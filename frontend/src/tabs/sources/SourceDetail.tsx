import { useEffect, useState } from "react";
import { api, errMsg } from "../../api";
import { cancelledLine } from "../../jobControlsText";
import { radius, theme } from "../../theme";
import type { ItemMeta, Job, Source } from "../../types";
import { SourceRow } from "./SourceRow";

const PAGE = 50;
const RECENT_JOBS = 8;

/** R49.9: one source's Library page (`#/library/<id>`): its controls, the
 *  documents it holds (GET /items, metadata only) and its recent jobs. */
export function SourceDetail({
  source,
  activeJobs,
  allJobs,
  onChanged,
  onBack,
}: {
  source: Source;
  activeJobs: Job[];
  allJobs: Job[];
  onChanged: () => void;
  onBack: () => void;
}) {
  const jobs = allJobs.filter((j) => j.source_id === source.id).slice(0, RECENT_JOBS);
  return (
    <div>
      <a
        href="#/library"
        onClick={(e) => {
          e.preventDefault();
          onBack();
        }}
        style={{ fontSize: 13, color: theme.textMuted }}
      >
        ← Library
      </a>
      <h2 style={{ marginTop: 8 }}>{source.name}</h2>
      <ul style={{ listStyle: "none", padding: 0, margin: "0 0 24px" }}>
        <SourceRow source={source} config={source.config ?? {}} jobs={activeJobs} onChanged={onChanged} />
      </ul>
      <Documents sourceId={source.id} syncedAt={source.last_synced_at} />
      <h3 style={styles.h3}>Recent jobs</h3>
      {jobs.length === 0 ? (
        <p style={styles.empty}>No jobs for this source yet.</p>
      ) : (
        <ul style={styles.list}>
          {jobs.map((j) => (
            <li key={j.id} style={styles.row}>
              <span style={{ fontWeight: 600, color: j.status === "failed" ? theme.red : theme.text }}>{j.kind} {j.status}</span>
              <span style={{ color: theme.textMuted }}> · {new Date(j.created_at).toLocaleString()}</span>
              <JobOutcome job={j} />
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

function JobOutcome({ job: j }: { job: Job }) {
  if (j.status === "failed" && j.error) return <div style={{ color: theme.red, fontSize: 12 }}>{j.error}</div>;
  if (j.status === "cancelled") return <div style={styles.meta}>{cancelledLine(j.processed, j.total)}</div>;
  if (j.status === "done" && j.result) return <div style={styles.meta}>{j.result.items ?? 0} documents, {j.result.entities ?? 0} records</div>;
  return <div style={styles.meta}>{j.processed}/{j.total}</div>;
}

function Documents({ sourceId, syncedAt }: { sourceId: number; syncedAt: string | null }) {
  const [items, setItems] = useState<ItemMeta[]>([]);
  const [total, setTotal] = useState<number | null>(null);
  const [limit, setLimit] = useState(PAGE);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    api
      .listItems({ source_id: sourceId, limit })
      .then((r) => {
        if (cancelled) return;
        setItems(r.items);
        setTotal(r.total);
      })
      .catch((e) => !cancelled && setError(errMsg(e)));
    return () => {
      cancelled = true;
    };
  }, [sourceId, limit, syncedAt]);

  return (
    <>
      <h3 style={styles.h3}>Documents{total !== null ? ` · ${total}` : ""}</h3>
      {error && <p style={{ color: theme.red, fontSize: 13 }}>{error}</p>}
      {total === 0 && <p style={styles.empty}>No documents yet. Sync the source to read it.</p>}
      <ul style={styles.list}>
        {items.map((it) => (
          <li key={it.id} style={styles.row}>
            <div style={{ fontWeight: 600 }}>
              {it.title || it.path}
              {it.sensitive_override && <span style={styles.tag}>Sensitive</span>}
              {it.is_glossary && <span style={styles.tag}>glossary</span>}
            </div>
            {it.title && it.path !== it.title && <div style={styles.path}>{it.path}</div>}
          </li>
        ))}
      </ul>
      {total !== null && items.length < total && (
        <button onClick={() => setLimit((l) => l + PAGE)} style={styles.more}>
          Show more ({total - items.length} left)
        </button>
      )}
    </>
  );
}

const styles: Record<string, React.CSSProperties> = {
  h3: { margin: "24px 0 8px" },
  empty: { color: theme.textDim, fontSize: 13 },
  list: { listStyle: "none", padding: 0, margin: 0, display: "grid", gap: 4 },
  row: { padding: "0.45rem 0.7rem", border: `1px solid ${theme.borderSoft}`, borderRadius: radius, background: theme.bgCard, fontSize: 13 },
  meta: { fontSize: 12, color: theme.textMuted },
  path: { fontFamily: theme.mono, fontSize: 12, color: theme.textMuted, overflowWrap: "anywhere" },
  tag: { marginLeft: 8, fontSize: 11, fontWeight: 600, color: theme.textMuted },
  more: { marginTop: 8, padding: "0.4rem 0.8rem", borderRadius: radius, border: `1px solid ${theme.border}`, background: theme.buttonBg, color: theme.text, cursor: "pointer", fontSize: 13 },
};
