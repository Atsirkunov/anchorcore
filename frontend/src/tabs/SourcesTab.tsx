import { useCallback, useEffect, useMemo, useState } from "react";
import { api } from "../api";
import { useJobsPoll } from "../hooks/useJobsPoll";
import { theme } from "../theme";
import { isActiveJob } from "../jobControlsText";
import type { Job, Source } from "../types";
import { ProjectSection } from "./sources/ProjectSection";
import { SourceForm } from "./sources/SourceForm";
import { SourceRow } from "./sources/SourceRow";
import { UnplacedImages } from "./sources/UnplacedImages";

const NO_JOBS: Job[] = [];

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
  // R30.4: every active job per source, oldest first — never a `.find`
  // (that picked the lowest-id job, so `Stop` could target the wrong one).
  const jobsBySource = useMemo(() => {
    const map = new Map<number, Job[]>();
    for (const j of (jobsQuery.data ?? []).filter(isActiveJob).sort((a, b) => a.id - b.id)) {
      map.set(j.source_id, [...(map.get(j.source_id) ?? []), j]);
    }
    return map;
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
      <UnplacedImages sources={sources} />
      <ul style={{ listStyle: "none", padding: 0, margin: 0, display: "grid", gap: 8 }}>
        {sources.map((s) => (
          <SourceRow
            key={s.id}
            source={s}
            config={configs[s.id] ?? {}}
            jobs={jobsBySource.get(s.id) ?? NO_JOBS}
            onChanged={refresh}
          />
        ))}
        {sources.length === 0 && <li style={{ color: theme.textDim }}>No sources yet. Add a folder to watch.</li>}
      </ul>
    </div>
  );
}
