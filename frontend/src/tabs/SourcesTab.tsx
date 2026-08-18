import { useCallback, useEffect, useMemo, useState } from "react";
import { api } from "../api";
import { useJobsPoll } from "../hooks/useJobsPoll";
import { theme } from "../theme";
import type { Source } from "../types";
import { ProjectSection } from "./sources/ProjectSection";
import { SourceForm } from "./sources/SourceForm";
import { SourceRow } from "./sources/SourceRow";

/** B37: slim composition — sources list, configs, project section and the
 * shared jobs poll. The god component's state/logic moved into SourceForm /
 * SourceRow / ProjectSection / useJobsPoll. */
export function SourcesTab() {
  const [sources, setSources] = useState<Source[]>([]);
  const [configs, setConfigs] = useState<Record<number, Record<string, string>>>({});
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(() => {
    api
      .listSources()
      .then((list) => {
        setSources(list);
        Promise.all(
          list.map((s) =>
            api.sourceConfig(s.id).then((cfg) => ({ id: s.id, cfg })).catch(() => null),
          ),
        )
          .then((all) => {
            const map: Record<number, Record<string, string>> = {};
            for (const entry of all) if (entry) map[entry.id] = entry.cfg;
            setConfigs(map);
          })
          .catch(() => {});
      })
      .catch((e) => setError(String(e)));
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  // one polling hook for ingestion jobs (pauses when nothing runs / tab hidden)
  const jobsQuery = useJobsPoll();
  const runningJobs = useMemo(() => {
    const jobs = jobsQuery.data ?? [];
    return Object.fromEntries(jobs.filter((j) => j.status === "running" || j.status === "pending").map((j) => [j.id, j]));
  }, [jobsQuery.data]);

  useEffect(() => {
    if (jobsQuery.data && jobsQuery.data.some((j) => j.status !== "running" && j.status !== "pending")) {
      refresh();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [jobsQuery.data]);

  return (
    <div>
      <h2>Sources</h2>
      <ProjectSection sources={sources} onChanged={refresh} />
      <SourceForm onAdded={refresh} />
      {error && <p style={{ color: theme.red }}>{error}</p>}
      <ul style={{ listStyle: "none", padding: 0, margin: 0, display: "grid", gap: 8 }}>
        {sources.map((s) => (
          <SourceRow
            key={s.id}
            source={s}
            config={configs[s.id] ?? {}}
            job={Object.values(runningJobs).find((j) => j.source_id === s.id)}
            onChanged={refresh}
          />
        ))}
        {sources.length === 0 && <li style={{ color: theme.textDim }}>No sources yet. Add a folder to watch.</li>}
      </ul>
    </div>
  );
}
