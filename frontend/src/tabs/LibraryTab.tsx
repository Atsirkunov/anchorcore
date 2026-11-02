import { useCallback, useEffect, useMemo, useState } from "react";
import { api, errMsg } from "../api";
import { useJobsPoll } from "../hooks/useJobsPoll";
import { radius, theme } from "../theme";
import { isActiveJob } from "../jobControlsText";
import type { Job, Source } from "../types";
import { ProjectSection } from "./sources/ProjectSection";
import { SourceDetail } from "./sources/SourceDetail";
import { SourceForm } from "./sources/SourceForm";
import { SourceRow } from "./sources/SourceRow";
import { UnplacedImages } from "./sources/UnplacedImages";

const NO_JOBS: Job[] = [];

/** R49.9: the Library (was Sources). The list comes first; "Add source"
 *  opens the form; projects sit below; a source opens its own page
 *  (`#/library/<id>`: documents, images, jobs). One jobs poll feeds both. */
export function LibraryTab({ sourceId, onOpen }: { sourceId: number | null; onOpen: (id: number | null) => void }) {
  const [sources, setSources] = useState<Source[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [adding, setAdding] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(() => {
    api
      .listSources()
      .then((list) => {
        setSources(list);
        setLoaded(true);
      })
      .catch((e) => setError(errMsg(e)));
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  // one polling hook for ingestion jobs (pauses when nothing runs / tab hidden)
  const jobsQuery = useJobsPoll();
  // R30.4: every active job per source, oldest first — never a `.find`.
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

  const open = sourceId !== null ? sources.find((s) => s.id === sourceId) : undefined;
  if (sourceId !== null && open) {
    return (
      <SourceDetail
        source={open}
        activeJobs={jobsBySource.get(open.id) ?? NO_JOBS}
        allJobs={jobsQuery.data ?? NO_JOBS}
        onChanged={refresh}
        onBack={() => onOpen(null)}
      />
    );
  }

  return (
    <div>
      <div style={styles.head}>
        <h2 style={{ margin: 0 }}>Library</h2>
        <button onClick={() => setAdding((a) => !a)} style={adding ? styles.secondary : styles.primary} aria-expanded={adding}>
          {adding ? "Close" : "+ Add source"}
        </button>
      </div>
      {sourceId !== null && loaded && <p style={{ color: theme.textDim }}>That source no longer exists.</p>}
      {adding && (
        <div style={styles.drawer}>
          <SourceForm
            onAdded={() => {
              refresh();
              setAdding(false);
            }}
          />
        </div>
      )}
      {error && <p style={{ color: theme.red }}>{error}</p>}
      <ul style={{ listStyle: "none", padding: 0, margin: "0 0 24px", display: "grid", gap: 8 }}>
        {sources.map((s) => (
          <SourceRow
            key={s.id}
            source={s}
            config={s.config ?? {}}
            jobs={jobsBySource.get(s.id) ?? NO_JOBS}
            onChanged={refresh}
            onOpen={() => onOpen(s.id)}
          />
        ))}
        {loaded && sources.length === 0 && !adding && (
          <li style={{ color: theme.textDim }}>
            No sources yet.{" "}
            <button onClick={() => setAdding(true)} style={styles.link}>
              Add a folder
            </button>{" "}
            to start.
          </li>
        )}
      </ul>
      <UnplacedImages sources={sources} />
      <ProjectSection sources={sources} onChanged={refresh} />
    </div>
  );
}

const styles: Record<string, React.CSSProperties> = {
  head: { display: "flex", alignItems: "center", justifyContent: "space-between", gap: 12, marginBottom: 16 },
  primary: { padding: "0.5rem 1rem", borderRadius: radius, border: "none", background: theme.accent, color: theme.onAccent, cursor: "pointer", fontSize: 13 },
  secondary: { padding: "0.5rem 1rem", borderRadius: radius, border: `1px solid ${theme.border}`, background: theme.buttonBg, color: theme.text, cursor: "pointer", fontSize: 13 },
  drawer: { border: `1px solid ${theme.border}`, borderRadius: radius, background: theme.bgCard, padding: "1rem", marginBottom: 16 },
  link: { background: "none", border: "none", padding: 0, color: theme.text, textDecoration: "underline", cursor: "pointer", fontSize: "inherit" },
};
