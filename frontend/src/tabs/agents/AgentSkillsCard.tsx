import { useEffect, useState } from "react";
import { api, errMsg } from "../../api";
import { theme, radius } from "../../theme";
import type { McpClient } from "../../types";
import { SkillHandleField } from "../../SkillHandleField";
import { handleError, initialHandle, installNote } from "../../skillHandle";

const SKILLS = [
  {
    name: "setup",
    title: "Setup skill",
    dir: "anchorcore-setup",
    file: "anchorcore-setup-SKILL.md",
    blurb: "Install, configure, and connect AnchorCore via an agent.",
  },
  {
    name: "search",
    title: "Ask docs skill (read-only search)",
    dir: "ask-docs",
    file: "ask-docs-SKILL.md",
    blurb: "Read-only retrieval for agents when MCP is blocked.",
  },
];

type SkillInstallClient = { id: string; name: string; detected: boolean; skills_dir: string | null; search_handles?: string[] };
// `handle` is set for the search skill only (R47.4) — it names the folder.
type PendingInstall = { skill: string; title: string; clientName: string; skillDir: string; handle?: string };

/** Agent skills (moved from System, R49.9): served by this app; Install is
 *  offered only when no MCP client is connected (R23.7). */
export function AgentSkillsCard() {
  const [copied, setCopied] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  // R23.7: Install is the blocked-fallback path — offered only when no MCP
  // client is connected (MCP-first routing), never alongside a working MCP.
  const [installClients, setInstallClients] = useState<SkillInstallClient[] | null>(null);
  const [mcpClients, setMcpClients] = useState<McpClient[] | null>(null);
  const [clientId, setClientId] = useState("");
  const [pending, setPending] = useState<PendingInstall | null>(null);
  const [installing, setInstalling] = useState(false);
  const [notes, setNotes] = useState<Record<string, string>>({});

  useEffect(() => {
    api
      .skillInstallTargets()
      .then((r) => {
        setInstallClients(r.clients);
        const usable = r.clients.filter((c) => c.skills_dir);
        const first = usable.find((c) => c.detected) ?? usable[0];
        if (first) setClientId(first.id);
      })
      .catch(() => {});
    api
      .mcpClients()
      .then((r) => setMcpClients(r.clients))
      .catch(() => {});
  }, []);

  const mcpWorking = mcpClients?.some((c) => c.connected) ?? false;
  const offerInstall = mcpClients !== null && !mcpWorking && installClients !== null;
  const selectedClient = installClients?.find((c) => c.id === clientId) ?? null;

  const targetPath = (skillDir: string) =>
    selectedClient?.skills_dir ? `${selectedClient.skills_dir}/${skillDir}/SKILL.md` : null;

  const install = (name: string, title: string, skillDir: string) => {
    if (!selectedClient || !targetPath(skillDir)) return;
    // Click = consent via confirm step stating the target path.
    const handle = name === "search" ? initialHandle(selectedClient.search_handles) : undefined;
    setPending({ skill: name, title, clientName: selectedClient.name, skillDir, handle });
  };
  const pendingPath = pending ? targetPath(pending.handle?.trim() || pending.skillDir) ?? "" : "";
  const pendingInvalid = pending?.handle !== undefined && handleError(pending.handle) !== null;

  const confirmInstall = async () => {
    if (!pending || !selectedClient) return;
    setInstalling(true);
    try {
      const r = await api.skillInstall(pending.skill, selectedClient.id, pending.handle?.trim());
      setNotes((n) => ({ ...n, [pending.skill]: installNote(r, pendingPath, pending.clientName) }));
    } catch (e) {
      setNotes((n) => ({ ...n, [pending.skill]: errMsg(e) }));
    } finally {
      setInstalling(false);
      setPending(null);
    }
  };

  const copy = async (name: string) => {
    setBusy(name);
    setError(null);
    try {
      const text = await api.fetchSkill(name);
      await navigator.clipboard?.writeText(text);
      setCopied(name);
      setTimeout(() => setCopied(null), 1500);
    } catch (e) {
      setError(errMsg(e));
    } finally {
      setBusy(null);
    }
  };

  const download = async (name: string, file: string) => {
    setBusy(name);
    setError(null);
    try {
      const text = await api.fetchSkill(name);
      const url = URL.createObjectURL(new Blob([text], { type: "text/markdown" }));
      const a = document.createElement("a");
      a.href = url;
      a.download = file;
      a.click();
      URL.revokeObjectURL(url);
    } catch (e) {
      setError(errMsg(e));
    } finally {
      setBusy(null);
    }
  };

  return (
    <div style={{ ...styles.card, marginBottom: 16 }}>
      <div style={styles.cardTitle}>Agent skills</div>
      <div style={{ fontSize: 12, color: theme.textMuted }}>
        Served by this app. An agent can fetch them over HTTP with no git clone.
      </div>
      {offerInstall && installClients && (
        <div style={{ display: "flex", gap: 8, marginTop: 8, alignItems: "center", flexWrap: "wrap" }}>
          <span style={{ fontSize: 12, color: theme.textMuted }}>
            MCP isn't connected. Install a skill file into an agent instead:
          </span>
          <select
            value={clientId}
            onChange={(e) => setClientId(e.target.value)}
            style={styles.select}
            aria-label="Agent to install the skill into"
          >
            {installClients.map((c) => (
              <option key={c.id} value={c.id} disabled={!c.skills_dir}>
                {c.name}
                {c.detected ? "" : " (not detected)"}
              </option>
            ))}
          </select>
        </div>
      )}
      {SKILLS.map((s) => (
        <div key={s.name} style={{ ...styles.row, marginTop: 8 }}>
          <div style={{ fontSize: 13, fontWeight: 600 }}>{s.title}</div>
          <div style={{ fontSize: 12, color: theme.textMuted }}>{s.blurb}</div>
          <div style={{ display: "flex", gap: 8, marginTop: 6, flexWrap: "wrap" }}>
            <button onClick={() => copy(s.name)} disabled={busy !== null} style={styles.button}>
              {copied === s.name ? "Copied!" : "Copy skill"}
            </button>
            <button
              onClick={() => download(s.name, s.file)}
              disabled={busy !== null}
              style={{ ...styles.button, background: theme.bgHover, border: `1px solid ${theme.border}` }}
            >
              Download SKILL.md
            </button>
            {offerInstall && (
              <button
                onClick={() => install(s.name, s.title, s.dir)}
                disabled={installing || !selectedClient?.skills_dir}
                title={selectedClient?.skills_dir ? `Install into ${selectedClient.name}` : "No install target"}
                style={{ ...styles.button, background: theme.accent, color: theme.onAccent }}
              >
                Install…
              </button>
            )}
          </div>
          {notes[s.name] && (
            <div style={{ fontSize: 11, color: theme.textMuted, marginTop: 4, wordBreak: "break-all" }}>{notes[s.name]}</div>
          )}
        </div>
      ))}
      {pending && (
        <div style={{ ...styles.row, marginTop: 8, borderColor: theme.amber }}>
          <div style={{ fontSize: 13, fontWeight: 600 }}>Install {pending.title} into {pending.clientName}?</div>
          <div style={{ fontSize: 12, color: theme.textMuted, wordBreak: "break-all" }}>
            This writes {pendingPath}. An identical file is left alone; a different file is backed up first, never
            overwritten blindly.
          </div>
          {pending.handle !== undefined && (
            <SkillHandleField value={pending.handle} disabled={installing} onChange={(h) => setPending({ ...pending, handle: h })} />
          )}
          <div style={{ display: "flex", gap: 8, marginTop: 6 }}>
            <button
              onClick={confirmInstall}
              disabled={installing || pendingInvalid}
              style={{ ...styles.button, background: theme.accent, color: theme.onAccent }}
            >
              {installing ? "Installing…" : "Confirm install"}
            </button>
            <button
              onClick={() => setPending(null)}
              disabled={installing}
              style={{ ...styles.button, background: theme.bgHover, border: `1px solid ${theme.border}` }}
            >
              Cancel
            </button>
          </div>
        </div>
      )}
      {error && <div style={{ fontSize: 12, color: theme.red, marginTop: 6 }}>{error}</div>}
    </div>
  );
}

const styles: Record<string, React.CSSProperties> = {
  button: { padding: "0.5rem 1rem", borderRadius: radius, border: "none", background: theme.buttonBg, color: theme.text, cursor: "pointer" },
  select: { padding: "0.5rem 0.75rem", borderRadius: radius, border: `1px solid ${theme.border}`, background: theme.bgCard, color: theme.text },
  card: { background: theme.bgCard, border: `1px solid ${theme.border}`, borderRadius: radius, padding: "0.7rem 0.9rem" },
  cardTitle: { fontWeight: 600, marginBottom: 4 },
  row: { display: "flex", flexDirection: "column", gap: 2, background: theme.bgCard, border: `1px solid ${theme.border}`, borderRadius: radius, padding: "0.5rem 0.75rem" },
  link: { color: theme.accentAlt, fontSize: 13, textDecoration: "none" },
};

