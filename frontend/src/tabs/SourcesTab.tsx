import { useCallback, useEffect, useState } from "react";
import { api } from "../api";
import type { Source } from "../types";

export function SourcesTab() {
  const [sources, setSources] = useState<Source[]>([]);
  const [connector, setConnector] = useState("folder");
  const [name, setName] = useState("");
  const [path, setPath] = useState("");
  const [jira, setJira] = useState({ base_url: "", email: "", token: "", project: "" });
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(() => {
    api.listSources().then(setSources).catch((e) => setError(String(e)));
  }, []);

  useEffect(() => refresh(), [refresh]);

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

  async function sync(id: number) {
    setError(null);
    try {
      await api.syncSource(id);
      refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
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
        {sources.map((s) => (
          <li key={s.id} style={{ display: "flex", alignItems: "center", justifyContent: "space-between", background: "#171a21", border: "1px solid #2d333b", borderRadius: 8, padding: "0.6rem 0.9rem" }}>
            <div>
              <div style={{ fontWeight: 600 }}>{s.name}</div>
              <div style={{ fontSize: 12, color: "#9ca3af" }}>
                {s.connector} · last sync: {s.last_synced_at ? new Date(s.last_synced_at).toLocaleString() : "never"}
              </div>
            </div>
            <div style={{ display: "flex", gap: 8 }}>
              <button onClick={() => sync(s.id)} style={styles.button}>Sync now</button>
              <button onClick={() => api.deleteSource(s.id).then(refresh)} style={{ ...styles.button, background: "#3a1d1d" }}>
                Delete
              </button>
            </div>
          </li>
        ))}
        {sources.length === 0 && <li style={{ color: "#6b7280" }}>No sources yet. Add a folder to watch.</li>}
      </ul>
    </div>
  );
}

const styles: Record<string, React.CSSProperties> = {
  input: { padding: "0.55rem 0.75rem", borderRadius: 8, border: "1px solid #2d333b", background: "#171a21", color: "#e6e8eb" },
  button: { padding: "0.55rem 1rem", borderRadius: 8, border: "none", background: "#2b3240", color: "#e6e8eb", cursor: "pointer" },
};
