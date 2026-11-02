import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { api, errMsg } from "../../api";
import { useJobsPoll } from "../../hooks/useJobsPoll";
import { cancelledLine, isActiveJob } from "../../jobControlsText";
import { JobStopButton, StopNoteLine } from "../../JobStopButton";
import { useJobStop, type StopNote } from "../../useJobStop";
import { useModelPull } from "../../useModelPull";
import { theme, radius } from "../../theme";
import type { Job, LogFile, SystemStatus } from "../../types";
import { ErrorsPanel } from "../system/ErrorsPanel";

const POLL_MS = 10000;

const RECENT_JOBS = 10;

/** Settings → Status & logs (R49.9, was the System tab): health cards that
 *  have something to say, errors, jobs and log files. Agent access lives in
 *  the Agents tab; backup and updates have their own Settings sections. */
export function StatusSection({ onConfigure, focusJobs }: { onConfigure?: () => void; focusJobs?: number } = {}) {
  const [status, setStatus] = useState<SystemStatus | null>(null);
  const [logs, setLogs] = useState<LogFile[]>([]);
  // R30.4: the shared jobs poll (1 s while anything is active) — every
  // active job is listed with its own Stop control, plus the recent ones.
  const jobsQuery = useJobsPoll();
  const jobs = useMemo(() => {
    const all = jobsQuery.data ?? [];
    return all.filter((j, i) => i < RECENT_JOBS || isActiveJob(j));
  }, [jobsQuery.data]);
  const stopper = useJobStop();
  const [sourceNames, setSourceNames] = useState<Record<number, string>>({});
  const jobsRef = useRef<HTMLHeadingElement>(null);
  useEffect(() => {
    if (focusJobs) jobsRef.current?.scrollIntoView?.({ behavior: "smooth", block: "start" });
  }, [focusJobs]);
  const [refreshKey, setRefreshKey] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const [startingOllama, setStartingOllama] = useState(false);
  const [ollamaMsg, setOllamaMsg] = useState<string | null>(null);
  const pull = useModelPull();

  const queryClient = useQueryClient();
  const refresh = useCallback(() => {
    // the jobs poll idles when nothing is active — pick up jobs started elsewhere
    void queryClient.invalidateQueries({ queryKey: ["jobs"] });
    api
      .systemStatus()
      .then(setStatus)
      .catch((e) => setError(errMsg(e)));
    api.systemLogs().then(setLogs).catch(() => {});
    api
      .listSources()
      .then((list) => setSourceNames(Object.fromEntries(list.map((s) => [s.id, s.name]))))
      .catch(() => {});
  }, [queryClient]);

  useEffect(() => {
    refresh();
    const timer = setInterval(refresh, POLL_MS);
    return () => clearInterval(timer);
  }, [refresh]);

  return (
    <div>
      <div style={{ display: "flex", gap: 8, marginBottom: 12, flexWrap: "wrap" }}>
        <button
          onClick={() => {
            refresh();
            setRefreshKey((k) => k + 1);
          }}
          style={styles.button}
        >
          Refresh
        </button>
      </div>
      {error && <p style={{ color: theme.red }}>{error}</p>}

      {status && (
        <div style={{ display: "grid", gridTemplateColumns: "repeat(auto-fit, minmax(220px, 1fr))", gap: 8, marginBottom: 16 }}>
          <div style={styles.card}>
            <div style={styles.cardTitle}>API</div>
            <div>version {status.version}</div>
            {/* The server masks the path as "redacted"; never print that as data. */}
            {status.database && status.database !== "redacted" ? (
              <div style={{ fontSize: 14, color: theme.textMuted, wordBreak: "break-all" }}>{status.database}</div>
            ) : null}
          </div>
          <div style={styles.card}>
            <div style={styles.cardTitle}>
              Ollama{" "}
              <span style={{ color: status.ollama.reachable ? theme.green : theme.red }}>
                {status.ollama.reachable ? "up" : "offline"}
              </span>
            </div>
            <div style={{ fontSize: 14 }}>embeddings: {status.ollama.embed_model}</div>
            {status.ollama.missing_models.length > 0 && (
              <div style={{ fontSize: 14, color: theme.red }}>
                missing: {status.ollama.missing_models.join(", ")}
              </div>
            )}
            {status.ollama.reachable && status.ollama.missing_models.length > 0 && !pull.pulling && (
              <button
                onClick={async () => {
                  const done = await pull.startPull(status.ollama.missing_models ?? []);
                  if (done.length > 0) refresh();
                }}
                style={{ ...styles.button, background: theme.accent, color: theme.onAccent, fontSize: 14, padding: "0.35rem 0.7rem", marginTop: 6 }}
              >
                Install missing
              </button>
            )}
            {pull.pulling && (
              <div style={{ fontSize: 14, marginTop: 6 }}>
                Installing… {pull.pulls.filter((p) => p.done).length}/{pull.pulls.length || status.ollama.missing_models.length}
              </div>
            )}
            {pull.pullError && <div style={{ fontSize: 14, color: theme.red, marginTop: 4 }}>{pull.pullError}</div>}
            <div style={{ display: "flex", gap: 6, marginTop: 8, flexWrap: "wrap" }}>
              {!status.ollama.reachable && (
                <button
                  onClick={async () => {
                    setStartingOllama(true);
                    setOllamaMsg(null);
                    try {
                      const res = await api.startOllama();
                      if (res.ok) {
                        setOllamaMsg(res.already_running ? "Ollama already running" : "Ollama started, refreshing…");
                        setTimeout(refresh, 1500);
                      } else {
                        setOllamaMsg(res.error ?? "Failed to start Ollama");
                      }
                    } catch (e) {
                      setOllamaMsg(errMsg(e));
                    } finally {
                      setStartingOllama(false);
                    }
                  }}
                  disabled={startingOllama}
                  style={{ ...styles.button, background: theme.accent, color: theme.onAccent, fontSize: 14, padding: "0.35rem 0.7rem", opacity: startingOllama ? 0.6 : 1 }}
                >
                  {startingOllama ? "Starting…" : "Start local LLM"}
                </button>
              )}
              <button
                onClick={() => onConfigure?.()}
                style={{ ...styles.button, background: theme.bgHover, border: `1px solid ${theme.border}`, fontSize: 14, padding: "0.35rem 0.7rem" }}
              >
                Configure model
              </button>
            </div>
            {ollamaMsg && <div style={{ fontSize: 14, color: theme.textMuted, marginTop: 6 }}>{ollamaMsg}</div>}
          </div>
          <div style={styles.card}>
            <div style={styles.cardTitle}>Answer model</div>
            <div style={{ fontSize: 14 }}>
              {status.answer.provider} · {status.answer.model}
              {status.answer_ready === false && <div style={{ color: theme.amber }}>not working: answers show matching passages only</div>}
            </div>
          </div>
          {status.pending_embeddings > 0 && (
            <div style={styles.card}>
              <div style={styles.cardTitle}>Embedding backfill</div>
              <div style={{ fontSize: 14, color: theme.amber }}>{status.pending_embeddings} chunks pending</div>
            </div>
          )}
          {status.retrieval.calls > 0 && (
            <div style={styles.card}>
              <div style={styles.cardTitle}>Retrieval latency</div>
              <div style={{ fontSize: 14 }}>
                avg {status.retrieval.avg_latency_ms} ms · {status.retrieval.calls} calls
                <div style={{ color: theme.textMuted }}>backend: {status.retrieval.backend}</div>
              </div>
            </div>
          )}
          <div style={styles.card}>
            <div style={styles.cardTitle}>Entity extraction</div>
            <div style={{ fontSize: 14 }}>
              {status.extraction?.mode === "off" ? (
                <span style={{ color: theme.textDim }}>off: no entities are stored</span>
              ) : (
                <>rules: decisions and actions read from record structure, no model</>
              )}
            </div>
          </div>
          <div style={styles.card}>
            <div style={styles.cardTitle}>Embeddings</div>
            <div style={{ fontSize: 14 }}>
              provider:{" "}
              <span style={{ color: status.embedder.provider === "cloud" ? theme.amber : theme.green }}>
                {status.embedder.provider}
              </span>{" "}
              · {status.embedder.model}
            </div>
          </div>
          <StoppedTasks tasks={status.tasks} />
          {status.failing_sources.length > 0 && (
            <div style={styles.card}>
              <div style={styles.cardTitle}>Failing sources</div>
              {status.failing_sources.map((s) => (
                <div key={s.id} style={{ fontSize: 14, color: theme.red }}>
                  {s.name} ×{s.count}: {s.error}
                </div>
              ))}
            </div>
          )}
        </div>
      )}

      <ErrorsPanel refreshKey={refreshKey} />

      <h3 ref={jobsRef} id="jobs" style={{ marginBottom: 8 }}>Jobs</h3>
      {jobs.length === 0 ? (
        <p style={{ color: theme.textDim, fontSize: 15 }}>No jobs yet.</p>
      ) : (
        <ul style={{ listStyle: "none", padding: 0, margin: 0, display: "grid", gap: 6, marginBottom: 20 }}>
          {jobs.map((j) => (
            <JobRow
              key={j.id}
              job={j}
              sourceName={sourceNames[j.source_id]}
              busy={stopper.inFlight.has(j.id)}
              onStop={stopper.stop}
              note={stopper.notes[j.id]}
            />
          ))}
        </ul>
      )}

      <h3 style={{ marginBottom: 8 }}>Logs</h3>
      <ul style={{ listStyle: "none", padding: 0, margin: 0, display: "grid", gap: 6 }}>
        {logs.length === 0 && <li style={{ color: theme.textDim, fontSize: 15 }}>No log files yet.</li>}
        {logs.map((f) => (
          <li key={f.name} style={{ ...styles.row, justifyContent: "space-between" }}>
            <div>
              <span style={{ fontSize: 15 }}>{f.name}</span>{" "}
              <span style={{ fontSize: 14, color: theme.textMuted }}>
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

/** Background tasks only show up here when one has stopped. */
function StoppedTasks({ tasks }: { tasks: Record<string, string> }) {
  const stopped = Object.entries(tasks).filter(([, state]) => state !== "running");
  if (stopped.length === 0) return null;
  return (
    <div style={styles.card}>
      <div style={styles.cardTitle}>Background tasks</div>
      {stopped.map(([name, state]) => (
        <div key={name} style={{ fontSize: 14, color: theme.red }}>
          {name}: {state}
        </div>
      ))}
    </div>
  );
}

function jobStatusColor(status: Job["status"]): string {
  if (status === "failed") return theme.red;
  if (status === "cancelled") return theme.amber;
  if (status === "running" || status === "pending") return theme.accentAlt;
  return theme.green;
}

/** R30.4: one job line; running/pending rows carry their own labeled Stop. */
function JobRow({ job: j, sourceName, busy, onStop, note }: {
  job: Job;
  sourceName?: string;
  busy: boolean;
  onStop: (job: Job) => void;
  note?: StopNote;
}) {
  return (
    <li style={styles.row}>
      <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", gap: 8, flexWrap: "wrap" }}>
        <div style={{ fontSize: 14, color: theme.textMuted }}>
          #{j.id} {j.kind}{sourceName ? ` · ${sourceName}` : ""} · {new Date(j.created_at).toLocaleString()} · {j.processed}/{j.total}
        </div>
        {isActiveJob(j) && <JobStopButton job={j} sourceName={sourceName} busy={busy} onStop={onStop} />}
      </div>
      <div style={{ fontSize: 15 }}>
        <span style={{ fontWeight: 600, color: jobStatusColor(j.status) }}>{j.status}</span>
        {j.status === "done" && j.result && (
          <span> · {j.result.items ?? 0} items, {j.result.entities ?? 0} entities{j.result.purged ? `, ${j.result.purged} purged` : ''}{j.result.unchanged ? `, ${j.result.unchanged} unchanged` : ''}</span>
        )}
        {j.status === "cancelled" && <span style={{ color: theme.textMuted }}> · {cancelledLine(j.processed, j.total)}</span>}
        {j.status === "failed" && j.error && <span style={{ color: theme.red }}>: {j.error}</span>}
      </div>
      {note && !note.ok && <StopNoteLine note={note} />}
    </li>
  );
}

const styles: Record<string, React.CSSProperties> = {
  button: { padding: "0.5rem 1rem", borderRadius: radius, border: "none", background: theme.buttonBg, color: theme.text, cursor: "pointer", fontSize: 15 },
  select: { padding: "0.5rem 0.75rem", borderRadius: radius, border: `1px solid ${theme.border}`, background: theme.bgCard, color: theme.text, fontSize: 15 },
  card: { background: theme.bgCard, border: `1px solid ${theme.border}`, borderRadius: radius, padding: "1rem 1.2rem" },
  cardTitle: { fontWeight: 600, marginBottom: 4 },
  row: { display: "flex", flexDirection: "column", gap: 2, background: theme.bgCard, border: `1px solid ${theme.border}`, borderRadius: radius, padding: "0.6rem 0.9rem" },
  link: { color: theme.accentAlt, fontSize: 15, textDecoration: "none" },
};
