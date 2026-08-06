import { useCallback, useEffect, useRef, useState } from "react";
import { api } from "../api";
import type { Job, Source } from "../types";

const POLL_MS = 1000;

export function SourcesTab() {
  const [sources, setSources] = useState<Source[]>([]);
  const [connector, setConnector] = useState("folder");
  const [name, setName] = useState("");
  const [path, setPath] = useState("");
  const [jira, setJira] = useState({ base_url: "", email: "", token: "", project: "" });
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [runningJobs, setRunningJobs] = useState<Record<number, Job>>({});
  const pollRef = useRef<number | null>(null);

  const refresh = useCallback(() => {
    api.listSources().then(setSources).catch((e) => setError(String(e)));
  }, []);

  const stopPolling = useCallback(() => {
    if (pollRef.current !== null) {
      window.clearInterval(pollRef.current);
      pollRef.current = null;
    }
  }, []);

  const pollRunningJobs = useCallback(() => {
    api.listJobs().then((jobs) => {
      const running = Object.fromEntries(jobs.filter((j) => j.status === "running").map((j) => [j.id, j]));
      setRunningJobs(running);
      const finished = jobs.some((j) => j.status !== "running");
      if (Object.keys(running).length === 0) {
        stopPolling();
        refresh();
      } else if (finished) {
        refresh();
      }
    }).catch(() => {});
  }, [refresh, stopPolling]);

  useEffect(() => {
    refresh();
    pollRunningJobs();
    pollRef.current = window.setInterval(pollRunningJobs, POLL_MS);
    return stopPolling;
  }, [refresh, pollRunningJobs, stopPolling]);

  async function addSource() {
    setBusy(true);
    setError(null);
    try {
      const config: Record<string, string> =
        connector === "folder"
          ? { path }
          : { base_url: jira.base_url, email: jira.email, token: jira.token, project: jira.project };
      await api.createSource({ connector, name, config });
      setName("");
      setPath("");
      setJira({ base_url: "", email: "", token: "", project: "" });
      refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  async function startJob(id: number, kind: "sync" | "reclassify") {
    setError(null);
    try {
      const job = kind === "sync" ? await api.syncSource(id) : await api.reclassifySource(id);
      setRunningJobs((prev) => ({ ...prev, [job.id]: job }));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  async function cancelJob(job: Job) {
    setError(null);
    try {
      await api.cancelJob(job.id);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  function runningJobFor(sourceId: number): Job | undefined {
    return Object.values(runningJobs).find((j) => j.source_id === sourceId);
  }

  function jobLabel(job: Job): string {
    const verb = job.kind === "reclassify" ? "Reclassifying" : "Syncing";
    if (job.total > 0) return `${verb}… ${job.processed}/${job.total}`;
    return `${verb}…`;
  }

  return (
    <div>
      <h2>Sources</h2>
      <div style={{ display: "grid", gap: 8, marginBottom: 16 }}>
        <select value={connector} onChange={(e) => setConnector(e.target.value)} style={styles.input}>
          <option value="folder">Local folder</option>
          <option value="jira">Jira</option>
        </select>
        <input style={styles.input} placeholder="Source name" value={name} onChange={(e) => setName(e.target.value)} />
        {connector === "folder" ? (
          <input style={styles.input} placeholder="Folder path (absolute)" value={path} onChange={(e) => setPath(e.target.value)} />
        ) : (
          <>
            <input style={styles.input} placeholder="Jira base URL, e.g. https://x.atlassian.net" value={jira.base_url} onChange={(e) => setJira({ ...jira, base_url: e.target.value })} />
            <input style={styles.input} placeholder="Email" value={jira.email} onChange={(e) => setJira({ ...jira, email: e.target.value })} />
            <input style={styles.input} type="password" placeholder="API token" value={jira.token} onChange={(e) => setJira({ ...jira, token: e.target.value })} />
            <input style={styles.input} placeholder="Project key, e.g. PM" value={jira.project} onChange={(e) => setJira({ ...jira, project: e.target.value })} />
          </>
        )}
        <button onClick={addSource} disabled={busy} style={styles.button}>
          {busy ? "Adding…" : "Add source"}
        </button>
      </div>
      {error && <p style={{ color: "#f87171" }}>{error}</p>}
      <ul style={{ listStyle: "none", padding: 0, margin: 0, display: "grid", gap: 8 }}>
        {sources.map((s) => {
          const job = runningJobFor(s.id);
          return (
            <li key={s.id} style={{ display: "flex", alignItems: "center", justifyContent: "space-between", background: "#171a21", border: "1px solid #2d333b", borderRadius: 8, padding: "0.6rem 0.9rem" }}>
              <div style={{ flex: 1, minWidth: 0 }}>
                <div style={{ fontWeight: 600 }}>
                  {s.name}
                  {s.error_count > 0 && (
                    <span style={{ marginLeft: 8, fontSize: 11, background: "#3a1d1d", color: "#fca5a5", padding: "0.1rem 0.5rem", borderRadius: 999, fontWeight: 700 }}>
                      sync failing ×{s.error_count}
                    </span>
                  )}
                </div>
                <div style={{ fontSize: 12, color: "#9ca3af" }}>
                  {s.connector} · last sync: {s.last_synced_at ? new Date(s.last_synced_at).toLocaleString() : "never"}
                </div>
                {job && (
                  <div style={{ marginTop: 6 }}>
                    <div style={{ fontSize: 12, color: "#7dd3fc" }}>{jobLabel(job)}</div>
                    {job.total > 0 && (
                      <div style={{ height: 6, borderRadius: 3, background: "#2d333b", marginTop: 4, overflow: "hidden" }}>
                        <div style={{ height: "100%", width: `${Math.min(100, (job.processed / job.total) * 100)}%`, background: "#38bdf8", transition: "width 0.3s" }} />
                      </div>
                    )}
                  </div>
                )}
                {s.last_error && (
                  <div style={{ fontSize: 12, color: "#f87171", marginTop: 2 }} title={s.last_error}>{s.last_error.slice(0, 120)}</div>
                )}
              </div>
              <div style={{ display: "flex", gap: 8 }}>
                <button onClick={() => startJob(s.id, "sync")} disabled={!!job} style={styles.button}>
                  {job?.kind === "sync" ? "Syncing…" : "Sync now"}
                </button>
                <button onClick={() => startJob(s.id, "reclassify")} disabled={!!job} style={styles.button}>
                  {job?.kind === "reclassify" ? "Reclassifying…" : "Reclassify"}
                </button>
                {job && (
                  <button onClick={() => cancelJob(job)} style={{ ...styles.button, background: "#3a1d1d", color: "#fca5a5" }} title="Stop this job">
                    Stop
                  </button>
                )}
                <button onClick={() => api.deleteSource(s.id).then(refresh)} disabled={!!job} style={{ ...styles.button, background: "#3a1d1d" }}>
                  Delete
                </button>
              </div>
            </li>
          );
        })}
        {sources.length === 0 && <li style={{ color: "#6b7280" }}>No sources yet. Add a folder to watch.</li>}
      </ul>
    </div>
  );
}

const styles: Record<string, React.CSSProperties> = {
  input: { padding: "0.55rem 0.75rem", borderRadius: 8, border: "1px solid #2d333b", background: "#171a21", color: "#e6e8eb" },
  button: { padding: "0.55rem 1rem", borderRadius: 8, border: "none", background: "#2b3240", color: "#e6e8eb", cursor: "pointer" },
};
