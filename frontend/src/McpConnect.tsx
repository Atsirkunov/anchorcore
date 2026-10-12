import { useCallback, useEffect, useState } from "react";
import { api } from "./api";
import { theme } from "./theme";
import type { McpClient, McpInfo } from "./types";

/**
 * R20.9 — "Agent access (MCP)" card. Used by SystemTab and the onboarding
 * wizard: one-click Connect/Disconnect for detected clients, copy-paste
 * fallback for everything else.
 */
export function McpConnectCard() {
  const [mcp, setMcp] = useState<McpInfo | null>(null);
  const [clients, setClients] = useState<McpClient[]>([]);
  const [busy, setBusy] = useState<string | null>(null);
  const [notes, setNotes] = useState<Record<string, string>>({});
  const [copied, setCopied] = useState<string | null>(null);

  const refresh = useCallback(() => {
    api.systemMcp().then(setMcp).catch(() => {});
    api.mcpClients().then((r) => setClients(r.clients)).catch(() => {});
  }, []);

  useEffect(() => {
    refresh();
    const timer = setInterval(refresh, 15000);
    return () => clearInterval(timer);
  }, [refresh]);

  const backendUrl = window.location.origin;
  const defaultBackend = backendUrl === "http://127.0.0.1:8000" || backendUrl === "http://localhost:8000";
  const env = defaultBackend ? undefined : { ANCHOR_BACKEND_URL: backendUrl };
  const command = mcp?.command ?? "";
  const args = mcp?.args ?? ["mcp"];
  const claudeLine = command ? `claude mcp add anchorcore -- "${command}" ${args.join(" ")}` : "";
  const mcpServersJson = command
    ? JSON.stringify(
        {
          mcpServers: {
            anchorcore: {
              command,
              args,
              ...(env ? { env } : {}),
            },
          },
        },
        null,
        2
      )
    : "";
  const opencodeJson = command
    ? JSON.stringify(
        {
          mcp: {
            servers: {
              anchorcore: {
                type: "local",
                command: [command, ...args],
                ...(env ? { environment: env } : {}),
              },
            },
          },
        },
        null,
        2
      )
    : "";

  const copy = (label: string, text: string) => {
    navigator.clipboard
      ?.writeText(text)
      .then(() => {
        setCopied(label);
        setTimeout(() => setCopied(null), 1500);
      })
      .catch(() => {});
  };

  const act = async (id: string, action: "connect" | "disconnect") => {
    setBusy(id);
    setNotes((n) => ({ ...n, [id]: "" }));
    try {
      const r = action === "connect" ? await api.mcpConnect(id) : await api.mcpDisconnect(id);
      const note = r.ok
        ? r.method === "cli"
          ? "Done — restart your agent to pick it up."
          : `Written to ${r.config_path ?? "config"} — restart your agent.`
        : r.error ?? "Failed";
      setNotes((n) => ({ ...n, [id]: note }));
    } catch (e) {
      setNotes((n) => ({ ...n, [id]: e instanceof Error ? e.message : String(e) }));
    } finally {
      setBusy(null);
      refresh();
    }
  };

  return (
    <div style={{ ...styles.card, marginBottom: 16 }}>
      <div style={styles.cardTitle}>Agent access (MCP)</div>
      <div style={{ fontSize: 13, marginBottom: 6 }}>
        Claude Code, Claude Desktop, Cursor, opencode, Codex — same memory, same citations, no browser.
      </div>
      {mcp === null || !command ? (
        <div style={{ fontSize: 12, color: theme.textMuted }}>Checking MCP mode…</div>
      ) : (
        <>
          {clients.map((c) => (
            <div key={c.id} style={styles.clientRow}>
              <div style={{ flex: 1, minWidth: 0 }}>
                <div style={{ fontSize: 13 }}>
                  {c.name}{" "}
                  <span style={{ fontSize: 11, color: c.connected ? theme.green : c.detected ? theme.textMuted : theme.textDim }}>
                    {c.connected ? "connected" : c.detected ? "detected" : "not detected"}
                  </span>
                </div>
                {notes[c.id] && (
                  <div style={{ fontSize: 11, color: theme.textMuted, marginTop: 2, wordBreak: "break-all" }}>{notes[c.id]}</div>
                )}
              </div>
              {c.connected ? (
                <button disabled={busy === c.id} onClick={() => act(c.id, "disconnect")} style={styles.clientButton}>
                  {busy === c.id ? "…" : "Disconnect"}
                </button>
              ) : (
                <button
                  disabled={busy === c.id || !c.detected}
                  title={c.detected ? "" : `Not found on this machine${c.config_path ? ` (looked for ${c.config_path})` : ""}`}
                  onClick={() => act(c.id, "connect")}
                  style={{ ...styles.clientButton, background: theme.accent, color: theme.onAccent, opacity: c.detected ? 1 : 0.5 }}
                >
                  {busy === c.id ? "…" : "Connect"}
                </button>
              )}
            </div>
          ))}

          <div style={{ fontSize: 12, color: theme.textMuted, margin: "10px 0 4px" }}>
            Manual setup — the exact command (runs this app in stdio MCP mode, no separate binary):
          </div>
          <div style={{ display: "flex", gap: 8, alignItems: "center", marginBottom: 6 }}>
            <code style={styles.code}>{claudeLine}</code>
            <button onClick={() => copy("cmd", claudeLine)} style={styles.clientButton}>
              {copied === "cmd" ? "Copied" : "Copy"}
            </button>
          </div>
          <details>
            <summary style={styles.summary}>Claude Desktop / Cursor config (JSON)</summary>
            <pre style={styles.pre}>{mcpServersJson}</pre>
            <button onClick={() => copy("cmdjson", mcpServersJson)} style={{ ...styles.clientButton, marginTop: 6 }}>
              {copied === "cmdjson" ? "Copied" : "Copy JSON"}
            </button>
          </details>
          <details>
            <summary style={styles.summary}>opencode config (mcp.servers, V2)</summary>
            <pre style={styles.pre}>{opencodeJson}</pre>
            <button onClick={() => copy("ocjson", opencodeJson)} style={{ ...styles.clientButton, marginTop: 6 }}>
              {copied === "ocjson" ? "Copied" : "Copy JSON"}
            </button>
          </details>
          <div style={{ fontSize: 12, color: theme.textMuted, marginTop: 6 }}>
            Tools: ask + search (read-only) · audited as component=mcp
            {mcp.calls > 0 && (
              <span>
                {" "}· {mcp.calls} {mcp.calls === 1 ? "call" : "calls"} audited
                {mcp.last_call_at ? `, last ${new Date(mcp.last_call_at).toLocaleString()}` : ""}
              </span>
            )}
          </div>
        </>
      )}
    </div>
  );
}

const styles: Record<string, React.CSSProperties> = {
  card: { background: theme.bgCard, border: `1px solid ${theme.border}`, borderRadius: 8, padding: "0.7rem 0.9rem" },
  cardTitle: { fontWeight: 600, marginBottom: 4 },
  clientRow: { display: "flex", alignItems: "center", gap: 8, padding: "0.35rem 0", borderBottom: `1px solid ${theme.border}` },
  clientButton: { padding: "0.35rem 0.7rem", borderRadius: 8, border: "none", background: theme.buttonBg, color: theme.text, cursor: "pointer", fontSize: 12, flexShrink: 0 },
  code: { flex: 1, fontSize: 12, background: theme.bgElevated, border: `1px solid ${theme.border}`, borderRadius: 6, padding: "0.5rem 0.75rem", wordBreak: "break-all" },
  pre: { margin: "6px 0 0", padding: "0.5rem 0.75rem", fontSize: 12, background: theme.bgElevated, border: `1px solid ${theme.border}`, borderRadius: 6, overflowX: "auto" },
  summary: { fontSize: 12, color: theme.textMuted, cursor: "pointer" },
};
