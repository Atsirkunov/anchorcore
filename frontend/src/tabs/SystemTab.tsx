import { useCallback, useEffect, useState } from "react";
import { api } from "../api";
import type { Job, LogFile, SystemEvent, SystemStatus } from "../types";

const POLL_MS = 10000;

export function SystemTab() {
  const [status, setStatus] = useState<SystemStatus | null>(null);
  const [events, setEvents] = useState<SystemEvent[]>([]);
  const [logs, setLogs] = useState<LogFile[]>([]);
  const [jobs, setJobs] = useState<Job[]>([]);
  const [componentFilter, setComponentFilter] = useState("");
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(() => {
    api
      .systemStatus()
      .then(setStatus)
      .catch((e) => setError(e instanceof Error ? e.message : String(e)));
    api
      .systemErrors({ component: componentFilter || undefined, limit: 50 })
      .then(setEvents)
      .catch(() => {});
    api.systemLogs().then(setLogs).catch(() => {});
    api.listJobs(undefined, 10).then(setJobs).catch(() => {});
  }, [componentFilter]);

  useEffect(() => {
    refresh();
    const timer = setInterval(refresh, POLL_MS);
    return () => clearInterval(timer);
  }, [refresh]);

  const levelColor = (level: string) =>
    level === "error" ? "#f87171" : level === "warning" ? "#fbbf24" : "#93c5fd";

  return (
    <div>
      <h2>System</h2>
      <div style={{ display: "flex", gap: 8, marginBottom: 12 }}>
        <button onClick={refresh} style={styles.button}>Refresh</button>
        <select value={componentFilter} onChange={(e) => setComponentFilter(e.target.value)} style={styles.select}>
          <option value="">All components</option>
          <option value="pipeline">pipeline</option>
          <option value="scheduler">scheduler</option>
          <option value="embedder">embedder</option>
          <option value="qa">qa</option>
          <option value="system">system</option>
        </select>
      </div>
      {error && <p style={{ color: "#f87171" }}>{error}</p>}

      {status && (
        <div style={{ display: "grid", gridTemplateColumns: "repeat(auto-fit, minmax(220px, 1fr))", gap: 8, marginBottom: 16 }}>
          <div style={styles.card}>
            <div style={styles.cardTitle}>API</div>
            <div>version {status.version}</div>
            <div style={{ fontSize: 12, color: "#9ca3af", wordBreak: "break-all" }}>{status.database}</div>
          </div>
          <div style={styles.card}>
            <div style={styles.cardTitle}>
              Ollama{" "}
              <span style={{ color: status.ollama.reachable ? "#4ade80" : "#f87171" }}>
                {status.ollama.reachable ? "up" : "offline"}
              </span>
            </div>
            <div style={{ fontSize: 12 }}>classifier: {status.ollama.classifier_model}</div>
            <div style={{ fontSize: 12 }}>embeddings: {status.ollama.embed_model}</div>
            {status.ollama.missing_models.length > 0 && (
              <div style={{ fontSize: 12, color: "#f87171" }}>
                missing: {status.ollama.missing_models.join(", ")}
              </div>
            )}
          </div>
          <div style={styles.card}>
            <div style={styles.cardTitle}>Answer model</div>
            <div style={{ fontSize: 12 }}>
              {status.answer.provider} · {status.answer.model}
            </div>
          </div>
          <div style={styles.card}>
            <div style={styles.cardTitle}>Embedding backfill</div>
            <div style={{ fontSize: 12 }}>
              {status.pending_embeddings > 0 ? (
                <span style={{ color: "#fbbf24" }}>{status.pending_embeddings} chunks pending</span>
              ) : (
                <span style={{ color: "#4ade80" }}>up to date</span>
              )}
            </div>
          </div>
          <div style={styles.card}>
            <div style={styles.cardTitle}>Classifier throughput</div>
            <div style={{ fontSize: 12 }}>
              {status.classifier.windows > 0 ? (
                <>
                  {status.classifier.windows} windows · avg {status.classifier.avg_latency_ms} ms
                  <div style={{ color: "#9ca3af" }}>concurrency {status.classifier.concurrency}</div>
                </>
              ) : (
                <span style={{ color: "#6b7280" }}>no windows classified yet</span>
              )}
            </div>
          </div>
          <div style={styles.card}>
            <div style={styles.cardTitle}>Scheduler tasks</div>
            <div style={{ fontSize: 12 }}>
              {Object.entries(status.tasks).map(([name, state]) => (
                <div key={name}>
                  {name}: <span style={{ color: state === "running" ? "#4ade80" : "#f87171" }}>{state}</span>
                </div>
              ))}
            </div>
          </div>
          <div style={styles.card}>
            <div style={styles.cardTitle}>Failing sources</div>
            {status.failing_sources.length === 0 ? (
              <div style={{ fontSize: 12, color: "#4ade80" }}>none</div>
            ) : (
              status.failing_sources.map((s) => (
                <div key={s.id} style={{ fontSize: 12, color: "#f87171" }}>
                  {s.name} ×{s.count}: {s.error}
                </div>
              ))
            )}
          </div>
        </div>
      )}

      <h3 style={{ marginBottom: 8 }}>Errors & warnings</h3>
      {events.length === 0 ? (
        <p style={{ color: "#6b7280", fontSize: 13 }}>No errors recorded.</p>
      ) : (
        <ul style={{ listStyle: "none", padding: 0, margin: 0, display: "grid", gap: 6, marginBottom: 20 }}>
          {events.map((e) => (
            <li key={e.id} style={{ ...styles.row, borderLeft: `3px solid ${levelColor(e.level)}` }}>
              <div>
                <span style={{ fontSize: 12, color: "#9ca3af" }}>{new Date(e.created_at).toLocaleString()}</span>{" "}
                <span style={{ fontSize: 12, background: "#1e2430", borderRadius: 4, padding: "0 0.35rem" }}>
                  {e.component}
                </span>{" "}
                {e.source_name && <span style={{ fontSize: 12, color: "#7dd3fc" }}>{e.source_name}</span>}
              </div>
              <div style={{ fontSize: 13 }}>{e.message}</div>
              {e.detail && <div style={{ fontSize: 12, color: "#9ca3af" }} title={e.detail}>{e.detail.slice(0, 200)}</div>}
            </li>
          ))}
        </ul>
      )}

      <h3 style={{ marginBottom: 8 }}>Recent jobs</h3>
      {jobs.length === 0 ? (
        <p style={{ color: "#6b7280", fontSize: 13 }}>No jobs yet.</p>
      ) : (
        <ul style={{ listStyle: "none", padding: 0, margin: 0, display: "grid", gap: 6, marginBottom: 20 }}>
          {jobs.map((j) => (
            <li key={j.id} style={{ ...styles.row, borderLeft: `3px solid ${j.status === "failed" ? "#f87171" : j.status === "running" ? "#7dd3fc" : "#4ade80"}` }}>
              <div style={{ fontSize: 12, color: "#9ca3af" }}>
                #{j.id} {j.kind} · {new Date(j.created_at).toLocaleString()} · {j.processed}/{j.total}
              </div>
              <div style={{ fontSize: 13 }}>
                {j.status}
                {j.status === "done" && j.result && (
                  <span> — {j.result.items ?? 0} items, {j.result.entities ?? 0} entities</span>
                )}
                {j.status === "failed" && j.error && <span style={{ color: "#f87171" }}> — {j.error}</span>}
              </div>
            </li>
          ))}
        </ul>
      )}

      <h3 style={{ marginBottom: 8 }}>Logs</h3>
      <ul style={{ listStyle: "none", padding: 0, margin: 0, display: "grid", gap: 6 }}>
        {logs.length === 0 && <li style={{ color: "#6b7280", fontSize: 13 }}>No log files yet.</li>}
        {logs.map((f) => (
          <li key={f.name} style={{ ...styles.row, justifyContent: "space-between" }}>
            <div>
              <span style={{ fontSize: 13 }}>{f.name}</span>{" "}
              <span style={{ fontSize: 12, color: "#9ca3af" }}>
                ({(f.size / 1024).toFixed(0)} KB, {new Date(f.modified).toLocaleString()})
              </span>
            </div>
            <a href={api.logDownloadUrl(f.name)} download style={styles.link}>
              Download
            </a>
          </li>
        ))}
      </ul>
    </div>
  );
}

const styles: Record<string, React.CSSProperties> = {
  button: { padding: "0.5rem 1rem", borderRadius: 8, border: "none", background: "#2b3240", color: "#e6e8eb", cursor: "pointer" },
  select: { padding: "0.5rem 0.75rem", borderRadius: 8, border: "1px solid #2d333b", background: "#171a21", color: "#e6e8eb" },
  card: { background: "#171a21", border: "1px solid #2d333b", borderRadius: 8, padding: "0.7rem 0.9rem" },
  cardTitle: { fontWeight: 600, marginBottom: 4 },
  row: { display: "flex", flexDirection: "column", gap: 2, background: "#171a21", border: "1px solid #2d333b", borderRadius: 6, padding: "0.5rem 0.75rem" },
  link: { color: "#7dd3fc", fontSize: 13, textDecoration: "none" },
};
