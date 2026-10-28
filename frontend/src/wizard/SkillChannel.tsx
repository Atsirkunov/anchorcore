import { useEffect, useState } from "react";
import { api } from "../api";
import { theme } from "../theme";
import { styles } from "./styles";
import { SkillHandleField } from "../SkillHandleField";
import { handleError, initialHandle, installNote } from "../skillHandle";

type Client = { id: string; name: string; detected: boolean; skills_dir: string | null; search_handles?: string[] };

const SKILL = "search";
const DEFAULT_URLS = ["http://127.0.0.1:8000", "http://localhost:8000"];

/** R45.6: the REST channel — install the read-only ask-docs skill into an
 * agent (explicit confirm naming the path, R47.4 handle), or copy/download it. */
export function SkillChannel() {
  const [clients, setClients] = useState<Client[] | null>(null);
  const [clientId, setClientId] = useState("");
  const [confirming, setConfirming] = useState(false);
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const [handle, setHandle] = useState("");

  useEffect(() => {
    api
      .skillInstallTargets()
      .then((r) => {
        setClients(r.clients);
        const usable = r.clients.filter((c) => c.skills_dir);
        const first = usable.find((c) => c.detected) ?? usable[0];
        if (first) setClientId(first.id);
      })
      .catch((e) => setNote(e instanceof Error ? e.message : String(e)));
  }, []);

  const client = clients?.find((c) => c.id === clientId) ?? null;
  const target = client?.skills_dir ? `${client.skills_dir}/${handle.trim() || initialHandle(client.search_handles)}/SKILL.md` : null;
  const origin = window.location.origin;
  const customUrl = !DEFAULT_URLS.includes(origin);

  async function install() {
    if (!client) return;
    setBusy(true);
    try {
      const r = await api.skillInstall(SKILL, client.id, handle.trim());
      setNote(installNote(r, target ?? "", client.name));
    } catch (e) {
      setNote(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
      setConfirming(false);
    }
  }

  async function copy() {
    try {
      await navigator.clipboard?.writeText(await api.fetchSkill(SKILL));
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch (e) {
      setNote(e instanceof Error ? e.message : String(e));
    }
  }

  return (
    <div style={styles.note}>
      <div style={{ fontWeight: 600, marginBottom: 4 }}>REST skill (ask-docs)</div>
      <div style={{ fontSize: 13, color: theme.textMuted }}>
        A read-only skill file: your agent calls this app&apos;s local HTTP API (search, whole files, outlines) — no MCP server needed.
      </div>
      {clients && (
        <div style={{ display: "flex", gap: 8, marginTop: 8, alignItems: "center", flexWrap: "wrap" }}>
          <select value={clientId} onChange={(e) => setClientId(e.target.value)} style={{ ...styles.input, flex: "0 1 auto" }} aria-label="Agent to install the skill into">
            {clients.map((c) => (
              <option key={c.id} value={c.id} disabled={!c.skills_dir}>
                {c.name}
                {c.detected ? "" : " (not detected)"}
              </option>
            ))}
          </select>
          <button style={{ ...styles.button, ...styles.primary }} disabled={!target || busy} onClick={() => {
              setHandle(initialHandle(client?.search_handles));
              setConfirming(true);
            }}>
            Install…
          </button>
          <button style={{ ...styles.button, ...styles.secondary }} onClick={copy}>
            {copied ? "Copied!" : "Copy skill"}
          </button>
          <a href={api.skillUrl(SKILL)} download="ask-docs-SKILL.md" style={{ fontSize: 13, color: theme.accentAlt }}>
            Download SKILL.md
          </a>
        </div>
      )}
      {confirming && target && client && (
        <div style={{ ...styles.note, marginTop: 8, background: theme.bgCard }}>
          <div style={{ fontWeight: 600 }}>Install the search skill into {client.name}?</div>
          <div style={{ fontSize: 12, color: theme.textMuted, wordBreak: "break-all" }}>
            This writes {target}. An identical file is left alone; a different one is backed up first.
          </div>
          <SkillHandleField value={handle} disabled={busy} onChange={setHandle} />
          <div style={{ display: "flex", gap: 8, marginTop: 6 }}>
            <button style={{ ...styles.button, ...styles.primary }} disabled={busy || handleError(handle) !== null} onClick={install}>
              {busy ? "Installing…" : "Confirm install"}
            </button>
            <button style={{ ...styles.button, ...styles.secondary }} disabled={busy} onClick={() => setConfirming(false)}>
              Cancel
            </button>
          </div>
        </div>
      )}
      {note && <div style={{ fontSize: 12, color: theme.textMuted, marginTop: 6, wordBreak: "break-all" }}>{note}</div>}
      <div style={{ ...styles.hint, marginTop: 8 }}>
        {customUrl ? (
          <>
            This app runs at <code>{origin}</code>, not the skill&apos;s default — give your agent{" "}
            <code>ANCHOR_BACKEND_URL={origin}</code>.
          </>
        ) : (
          <>The skill finds this app at its default address, <code>{origin}</code>.</>
        )}{" "}
        If your agent&apos;s sandbox blocks localhost, allow it there.
      </div>
    </div>
  );
}
