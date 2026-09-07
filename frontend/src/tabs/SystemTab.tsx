import { useCallback, useEffect, useState } from "react";
import { api } from "../api";
import { theme } from "../theme";
import type { Job, LogFile, SystemEvent, SystemStatus } from "../types";

const POLL_MS = 10000;

export function SystemTab({ onConfigure }: { onConfigure?: () => void } = {}) {
  const [status, setStatus] = useState<SystemStatus | null>(null);
  const [events, setEvents] = useState<SystemEvent[]>([]);
  const [logs, setLogs] = useState<LogFile[]>([]);
  const [jobs, setJobs] = useState<Job[]>([]);
  const [componentFilter, setComponentFilter] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [startingOllama, setStartingOllama] = useState(false);
  const [ollamaMsg, setOllamaMsg] = useState<string | null>(null);

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
    level === "error" ? theme.red : level === "warning" ? theme.amber : theme.blue;

  return (
    <div>
      <h2>System</h2>
      <div style={{ display: "flex", gap: 8, marginBottom: 12, flexWrap: "wrap" }}>
        <button onClick={refresh} style={styles.button}>Refresh</button>
        <button
          onClick={() => {
            localStorage.removeItem("wizard-dismissed");
            location.reload();
          }}
          title="Re-open the welcome wizard (clears Skip)"
          style={{ ...styles.button, background: theme.bgHover, border: `1px solid ${theme.border}` }}
        >
          Show welcome wizard
        </button>
        <select value={componentFilter} onChange={(e) => setComponentFilter(e.target.value)} style={styles.select}>
          <option value="">All components</option>
          <option value="pipeline">pipeline</option>
          <option value="scheduler">scheduler</option>
          <option value="embedder">embedder</option>
          <option value="qa">qa</option>
          <option value="system">system</option>
        </select>
      </div>
      {error && <p style={{ color: theme.red }}>{error}</p>}

      {status && (
        <div style={{ display: "grid", gridTemplateColumns: "repeat(auto-fit, minmax(220px, 1fr))", gap: 8, marginBottom: 16 }}>
          <div style={styles.card}>
            <div style={styles.cardTitle}>API</div>
            <div>version {status.version}</div>
            <div style={{ fontSize: 12, color: theme.textMuted, wordBreak: "break-all" }}>{status.database}</div>
          </div>
          <div style={styles.card}>
            <div style={styles.cardTitle}>
              Ollama{" "}
              <span style={{ color: status.ollama.reachable ? theme.green : theme.red }}>
                {status.ollama.reachable ? "up" : "offline"}
              </span>
            </div>
            <div style={{ fontSize: 12 }}>classifier: {status.ollama.classifier_model}</div>
            <div style={{ fontSize: 12 }}>embeddings: {status.ollama.embed_model}</div>
            {status.ollama.missing_models.length > 0 && (
              <div style={{ fontSize: 12, color: theme.red }}>
                missing: {status.ollama.missing_models.join(", ")}
              </div>
            )}
            <div style={{ display: "flex", gap: 6, marginTop: 8, flexWrap: "wrap" }}>
              {!status.ollama.reachable && (
                <button
                  onClick={async () => {
                    setStartingOllama(true);
                    setOllamaMsg(null);
                    try {
                      const res = await api.startOllama();
                      if (res.ok) {
                        setOllamaMsg(res.already_running ? "Ollama already running" : "Ollama started — refreshing…");
                        setTimeout(refresh, 1500);
                      } else {
                        setOllamaMsg(res.error ?? "Failed to start Ollama");
                      }
                    } catch (e) {
                      setOllamaMsg(e instanceof Error ? e.message : String(e));
                    } finally {
                      setStartingOllama(false);
                    }
                  }}
                  disabled={startingOllama}
                  style={{ ...styles.button, background: theme.accent, color: theme.onAccent, fontSize: 12, padding: "0.35rem 0.7rem", opacity: startingOllama ? 0.6 : 1 }}
                >
                  {startingOllama ? "Starting…" : "Start local LLM"}
                </button>
              )}
              <button
                onClick={() => {
                  if (onConfigure) onConfigure();
                  else window.dispatchEvent(new CustomEvent("anchorcore:openSettings"));
                }}
                style={{ ...styles.button, background: theme.bgHover, border: `1px solid ${theme.border}`, fontSize: 12, padding: "0.35rem 0.7rem" }}
              >
                Configure model
              </button>
            </div>
            {ollamaMsg && <div style={{ fontSize: 12, color: theme.textMuted, marginTop: 6 }}>{ollamaMsg}</div>}
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
                <span style={{ color: theme.amber }}>{status.pending_embeddings} chunks pending</span>
              ) : (
                <span style={{ color: theme.green }}>up to date</span>
              )}
            </div>
          </div>
          <div style={styles.card}>
            <div style={styles.cardTitle}>Retrieval latency</div>
            <div style={{ fontSize: 12 }}>
              {status.retrieval.calls > 0 ? (
                <>
                  avg {status.retrieval.avg_latency_ms} ms · {status.retrieval.calls} calls
                  <div style={{ color: theme.textMuted }}>
                    backend:{" "}
                    <span style={{ color: status.retrieval.backend === "vec0" ? theme.green : status.retrieval.backend === "scan" ? theme.amber : theme.textDim }}>
                      {status.retrieval.backend}
                    </span>{" "}
                    · vec0 {status.retrieval.vec0_calls}
                  </div>
                </>
              ) : (
                <span style={{ color: theme.textDim }}>no retrieval yet</span>
              )}
            </div>
          </div>
          <div style={styles.card}>
            <div style={styles.cardTitle}>Classifier throughput</div>
            <div style={{ fontSize: 12 }}>
              {status.classifier.windows > 0 ? (
                <>
                  {status.classifier.windows} windows · avg {status.classifier.avg_latency_ms} ms
                  <div style={{ color: theme.textMuted }}>concurrency {status.classifier.concurrency}</div>
                </>
              ) : (
                <span style={{ color: theme.textDim }}>no windows classified yet</span>
              )}
            </div>
            <div style={{ fontSize: 12, marginTop: 4 }}>
              provider:{" "}
              <span style={{ color: status.classifier.provider === "cloud" ? theme.amber : theme.green }}>
                {status.classifier.provider}
              </span>{" "}
              · {status.classifier.model}
            </div>
          </div>
          <div style={styles.card}>
            <div style={styles.cardTitle}>Embeddings</div>
            <div style={{ fontSize: 12 }}>
              provider:{" "}
              <span style={{ color: status.embedder.provider === "cloud" ? theme.amber : theme.green }}>
                {status.embedder.provider}
              </span>{" "}
              · {status.embedder.model}
            </div>
          </div>
          <div style={styles.card}>
            <div style={styles.cardTitle}>Scheduler tasks</div>
            <div style={{ fontSize: 12 }}>
              {Object.entries(status.tasks).map(([name, state]) => (
                <div key={name}>
                  {name}: <span style={{ color: state === "running" ? theme.green : theme.red }}>{state}</span>
                </div>
              ))}
            </div>
          </div>
          <div style={styles.card}>
            <div style={styles.cardTitle}>Failing sources</div>
            {status.failing_sources.length === 0 ? (
              <div style={{ fontSize: 12, color: theme.green }}>none</div>
            ) : (
              status.failing_sources.map((s) => (
                <div key={s.id} style={{ fontSize: 12, color: theme.red }}>
                  {s.name} ×{s.count}: {s.error}
                </div>
              ))
            )}
          </div>
        </div>
      )}

      <div style={{ ...styles.card, marginBottom: 16 }}>
        <div style={styles.cardTitle}>Agent access (MCP)</div>
        <div style={{ fontSize: 13, marginBottom: 6 }}>
          Claude Code, Codex, opencode, Cursor — same memory, same citations, no browser.
        </div>
        <code style={{ display: "block", fontSize: 12, background: theme.bgElevated, border: `1px solid ${theme.border}`, borderRadius: 6, padding: "0.5rem 0.75rem", marginBottom: 6, wordBreak: "break-all" }}>
          ANCHOR_BACKEND_URL=http://127.0.0.1:8000 cargo run -p anchorcore --bin anchorcore-mcp
        </code>
        <div style={{ fontSize: 12, color: theme.textMuted }}>
          Tools: ask + search (read-only) · respects PII gates · audited as component=mcp
        </div>
      </div>

      <h3 style={{ marginBottom: 8 }}>Errors & warnings</h3>
      {events.length === 0 ? (
        <p style={{ color: theme.textDim, fontSize: 13 }}>No errors recorded.</p>
      ) : (
        <ul style={{ listStyle: "none", padding: 0, margin: 0, display: "grid", gap: 6, marginBottom: 20 }}>
          {events.map((e) => (
            <li key={e.id} style={{ ...styles.row, borderLeft: `3px solid ${levelColor(e.level)}` }}>
              <div>
                <span style={{ fontSize: 12, color: theme.textMuted }}>{new Date(e.created_at).toLocaleString()}</span>{" "}
                <span style={{ fontSize: 12, background: theme.bgHover, borderRadius: 4, padding: "0 0.35rem" }}>
                  {e.component}
                </span>{" "}
                {e.source_name && <span style={{ fontSize: 12, color: theme.accentAlt }}>{e.source_name}</span>}
              </div>
              <div style={{ fontSize: 13 }}>{e.message}</div>
              {e.detail && <div style={{ fontSize: 12, color: theme.textMuted }} title={e.detail}>{e.detail.slice(0, 200)}</div>}
            </li>
          ))}
        </ul>
      )}

      <h3 style={{ marginBottom: 8 }}>Recent jobs</h3>
      {jobs.length === 0 ? (
        <p style={{ color: theme.textDim, fontSize: 13 }}>No jobs yet.</p>
      ) : (
        <ul style={{ listStyle: "none", padding: 0, margin: 0, display: "grid", gap: 6, marginBottom: 20 }}>
          {jobs.map((j) => (
            <li key={j.id} style={{ ...styles.row, borderLeft: `3px solid ${j.status === "failed" ? theme.red : j.status === "cancelled" ? theme.amber : j.status === "running" ? theme.accentAlt : theme.green}` }}>
              <div style={{ fontSize: 12, color: theme.textMuted }}>
                #{j.id} {j.kind} · {new Date(j.created_at).toLocaleString()} · {j.processed}/{j.total}
              </div>
              <div style={{ fontSize: 13 }}>
                {j.status}
                {j.status === "done" && j.result && (
                  <span> — {j.result.items ?? 0} items, {j.result.entities ?? 0} entities</span>
                )}
                {j.status === "failed" && j.error && <span style={{ color: theme.red }}> — {j.error}</span>}
              </div>
            </li>
          ))}
        </ul>
      )}

      <h3 style={{ marginBottom: 8 }}>Logs</h3>
      <ul style={{ listStyle: "none", padding: 0, margin: 0, display: "grid", gap: 6 }}>
        {logs.length === 0 && <li style={{ color: theme.textDim, fontSize: 13 }}>No log files yet.</li>}
        {logs.map((f) => (
          <li key={f.name} style={{ ...styles.row, justifyContent: "space-between" }}>
            <div>
              <span style={{ fontSize: 13 }}>{f.name}</span>{" "}
              <span style={{ fontSize: 12, color: theme.textMuted }}>
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
  button: { padding: "0.5rem 1rem", borderRadius: 8, border: "none", background: theme.buttonBg, color: theme.text, cursor: "pointer" },
  select: { padding: "0.5rem 0.75rem", borderRadius: 8, border: `1px solid ${theme.border}`, background: theme.bgCard, color: theme.text },
  card: { background: theme.bgCard, border: `1px solid ${theme.border}`, borderRadius: 8, padding: "0.7rem 0.9rem" },
  cardTitle: { fontWeight: 600, marginBottom: 4 },
  row: { display: "flex", flexDirection: "column", gap: 2, background: theme.bgCard, border: `1px solid ${theme.border}`, borderRadius: 6, padding: "0.5rem 0.75rem" },
  link: { color: theme.accentAlt, fontSize: 13, textDecoration: "none" },
};
