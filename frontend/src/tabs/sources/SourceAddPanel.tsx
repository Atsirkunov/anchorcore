import { useEffect, useRef, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { api, errMsg } from "../../api";
import { FolderPicker } from "../../FolderPicker";
import { theme, commonStyles } from "../../theme";
import type { Source } from "../../types";
import { JiraProjectPicker, RestFields } from "./SourceForm";
import { REST_DEFAULTS } from "./restDefaults";

const CONNECTORS = [
  { value: "folder", label: "Folder on this computer", section: "Folder" },
  { value: "jira", label: "Jira", section: "Jira" },
  { value: "linear", label: "Linear", section: "Linear" },
  { value: "miro", label: "Miro", section: "Miro" },
  { value: "gdrive", label: "Google Drive", section: "Google Drive" },
  { value: "rest", label: "Generic REST API", section: "Generic REST API" },
] as const;

/** "Who may see it" — the sensitivity radio. Internal names allowed agents
 *  because MCP/skill callers need the person's AI-agents switch whatever
 *  the label (R35.5 caller rule); Sensitive keeps the proposal's wording. */
const LABELS: { value: Source["label"]; title: string; hint: string }[] = [
  { value: "internal", title: "Internal", hint: "You, your models and allowed agents." },
  { value: "public", title: "Public", hint: "Also safe for public-only searches." },
  { value: "sensitive", title: "Sensitive", hint: "Stays with local models. Cloud models and agents only if you allow it." },
];

const CLOSE_MS = 150;
const EASE = "cubic-bezier(0.22, 1, 0.36, 1)";

function prefersReducedMotion(): boolean {
  return typeof window.matchMedia === "function" && window.matchMedia("(prefers-reduced-motion: reduce)").matches;
}

/** UX proposal §Library: "Add source" opens a side drawer with a visible
 *  label on every field. Drawer motion follows the site rules: 250 ms open,
 *  150 ms close, no motion under Reduce Motion (then close is instant). */
export function SourceAddPanel({ onClose, onAdded }: { onClose: () => void; onAdded: (syncError: string | null) => void }) {
  const [closing, setClosing] = useState(false);
  const timer = useRef<number | null>(null);

  const close = () => {
    if (closing) return;
    if (prefersReducedMotion()) {
      onClose();
      return;
    }
    setClosing(true);
    timer.current = window.setTimeout(onClose, CLOSE_MS);
  };
  // The Escape listener stays mounted for the panel's life: re-subscribing
  // on `closing` would run the cleanup and cancel the close timer itself.
  const closeRef = useRef(close);
  useEffect(() => {
    closeRef.current = close;
  });

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") closeRef.current();
    };
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("keydown", onKey);
      if (timer.current !== null) window.clearTimeout(timer.current);
    };
  }, []);

  return (
    <div
      className="animated"
      style={{ ...styles.overlay, animation: closing ? `fadeOut ${CLOSE_MS}ms ease both` : `fadeIn 0.25s ease both` }}
      onClick={close}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label="Add source"
        className="animated"
        style={{ ...styles.panel, animation: closing ? `drawerOut ${CLOSE_MS}ms ${EASE} both` : `drawerIn 0.25s ${EASE} both` }}
        onClick={(e) => e.stopPropagation()}
      >
        <div style={styles.head}>
          <h2 style={{ margin: 0 }}>Add source</h2>
          <button onClick={close} style={styles.x} aria-label="Close" title="Close">
            ×
          </button>
        </div>
        <AddSourceForm
          onDone={(syncError) => {
            onAdded(syncError);
            close();
          }}
        />
      </div>
    </div>
  );
}

function AddSourceForm({ onDone }: { onDone: (syncError: string | null) => void }) {
  const [connector, setConnector] = useState<string>("folder");
  const [name, setName] = useState("");
  const [path, setPath] = useState("");
  const [pickerOpen, setPickerOpen] = useState(false);
  const [label, setLabel] = useState<Source["label"]>("internal");
  const [jira, setJira] = useState({ base_url: "", email: "", token: "", project: "" });
  const [gdrive, setGdrive] = useState({ folder_id: "", token: "" });
  const [linear, setLinear] = useState({ api_key: "", team: "" });
  const [miro, setMiro] = useState({ token: "", boards: "", team_id: "" });
  const [rest, setRest] = useState<Record<string, string>>({ ...REST_DEFAULTS });
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const queryClient = useQueryClient();
  const section = CONNECTORS.find((c) => c.value === connector)?.section ?? "Folder";

  async function addAndSync() {
    setBusy(true);
    setError(null);
    try {
      const config: Record<string, string> =
        connector === "folder"
          ? { path }
          : connector === "gdrive"
            ? { folder_id: gdrive.folder_id, token: gdrive.token }
            : connector === "linear"
              ? { api_key: linear.api_key, team: linear.team }
              : connector === "miro"
                ? { token: miro.token, boards: miro.boards, team_id: miro.team_id }
                : connector === "rest"
                  ? { ...rest }
                  : { base_url: jira.base_url, email: jira.email, token: jira.token, project: jira.project };
      // Creating never syncs on its own (sources.rs inserts the row only),
      // so the button starts the first sync itself; the jobs poll shows it.
      const created = await api.createSource({ connector, name, config, label });
      try {
        await api.syncSource(created.id);
        await queryClient.invalidateQueries({ queryKey: ["jobs"] });
        onDone(null);
      } catch (e) {
        // The source exists, so a failed sync start must not strand the
        // panel: close it and let the Library error line carry the message.
        onDone(errMsg(e));
      }
    } catch (e) {
      setError(errMsg(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div style={{ display: "grid", gap: 16 }}>
      <div>
        <label style={styles.field} htmlFor="add-source-type">
          Type
        </label>
        <select id="add-source-type" autoFocus value={connector} onChange={(e) => setConnector(e.target.value)} style={{ ...commonStyles.input, width: "100%", boxSizing: "border-box" }}>
          {CONNECTORS.map((c) => (
            <option key={c.value} value={c.value}>
              {c.label}
            </option>
          ))}
        </select>
      </div>
      <div>
        <span style={styles.field}>{section}</span>
        {connector === "folder" ? (
          <>
            <div style={{ display: "flex", gap: 8 }}>
              <input
                style={{ ...commonStyles.input, flex: 1, minWidth: 0 }}
                placeholder="Folder path (absolute)"
                aria-label="Folder path"
                value={path}
                onChange={(e) => setPath(e.target.value)}
              />
              <button type="button" style={commonStyles.button} title="Browse folders on this machine" onClick={() => setPickerOpen((v) => !v)}>
                Browse
              </button>
            </div>
            {pickerOpen && (
              <div style={{ marginTop: 8 }}>
                <FolderPicker
                  initialPath={path}
                  onSelect={(p) => {
                    setPath(p);
                    setPickerOpen(false);
                  }}
                  onCancel={() => setPickerOpen(false)}
                />
              </div>
            )}
          </>
        ) : connector === "gdrive" ? (
          <div style={{ display: "grid", gap: 8 }}>
            <input style={commonStyles.input} placeholder="Drive folder ID (from URL …/folders/…)" value={gdrive.folder_id} onChange={(e) => setGdrive({ ...gdrive, folder_id: e.target.value })} />
            <input style={commonStyles.input} type="password" placeholder="OAuth access token (Bearer)" value={gdrive.token} onChange={(e) => setGdrive({ ...gdrive, token: e.target.value })} />
            <div style={{ fontSize: 12, color: theme.textDim }}>Needs a Google Cloud OAuth token. See docs/sample-dataset.md or hosting/README for setup. Token is stored in OS keychain.</div>
          </div>
        ) : connector === "linear" ? (
          <div style={{ display: "grid", gap: 8 }}>
            <input style={commonStyles.input} type="password" placeholder="Linear API key (lin_api_...)" value={linear.api_key} onChange={(e) => setLinear({ ...linear, api_key: e.target.value })} />
            <input style={commonStyles.input} placeholder="Team key, e.g. ENG or ENG,PM" value={linear.team} onChange={(e) => setLinear({ ...linear, team: e.target.value })} />
            <div style={{ fontSize: 12, color: theme.textDim }}>Find API key at linear.app/settings/api · Team key is the 2–3 letter prefix (e.g. ENG). Token stored in OS keychain.</div>
          </div>
        ) : connector === "miro" ? (
          <div style={{ display: "grid", gap: 8 }}>
            <input style={commonStyles.input} type="password" placeholder="Miro access token" value={miro.token} onChange={(e) => setMiro({ ...miro, token: e.target.value })} />
            <input style={commonStyles.input} placeholder="Board IDs, comma-separated (from …/app/board/<id>/)" value={miro.boards} onChange={(e) => setMiro({ ...miro, boards: e.target.value })} />
            <input style={commonStyles.input} placeholder="or Team ID (every board of the team)" value={miro.team_id} onChange={(e) => setMiro({ ...miro, team_id: e.target.value })} />
            <div style={{ fontSize: 12, color: theme.textDim }}>Create a token in a Miro app with boards:read · Each board becomes one document (frames → sections, connectors → steps). Token stored in OS keychain.</div>
          </div>
        ) : connector === "rest" ? (
          <RestFields value={rest} onChange={setRest} />
        ) : (
          <div style={{ display: "grid", gap: 8 }}>
            <input style={commonStyles.input} placeholder="Jira base URL, e.g. https://x.atlassian.net" value={jira.base_url} onChange={(e) => setJira({ ...jira, base_url: e.target.value })} />
            <input style={commonStyles.input} placeholder="Email" value={jira.email} onChange={(e) => setJira({ ...jira, email: e.target.value })} />
            <input style={commonStyles.input} type="password" placeholder="API token" value={jira.token} onChange={(e) => setJira({ ...jira, token: e.target.value })} />
            <input style={commonStyles.input} placeholder="Project keys, e.g. PM or PM,TEST (or pick below)" value={jira.project} onChange={(e) => setJira({ ...jira, project: e.target.value })} />
            <JiraProjectPicker base_url={jira.base_url} email={jira.email} token={jira.token} project={jira.project} onChange={(v) => setJira({ ...jira, project: v })} />
          </div>
        )}
      </div>
      <div>
        <label style={styles.field} htmlFor="add-source-name">
          Name
        </label>
        <input id="add-source-name" style={{ ...commonStyles.input, width: "100%", boxSizing: "border-box" }} placeholder="e.g. Handbook" value={name} onChange={(e) => setName(e.target.value)} />
      </div>
      <div role="radiogroup" aria-label="Who may see it">
        <span style={styles.field}>Who may see it</span>
        <div style={{ display: "grid", gap: 12 }}>
          {LABELS.map((o) => (
            <label key={o.value} style={{ display: "flex", gap: 10, alignItems: "flex-start", cursor: "pointer" }}>
              <input type="radio" name="source-label" value={o.value} checked={label === o.value} onChange={() => setLabel(o.value)} style={{ marginTop: 4 }} />
              <span>
                <span style={{ display: "block", fontWeight: 700, fontSize: 14 }}>{o.title}</span>
                <span style={{ display: "block", fontSize: 13, color: theme.textDim }}>{o.hint}</span>
              </span>
            </label>
          ))}
        </div>
      </div>
      <button
        onClick={addAndSync}
        disabled={busy}
        style={{ ...commonStyles.button, background: theme.accent, color: theme.onAccent, borderColor: theme.accent, fontWeight: 600, width: "100%", boxSizing: "border-box", padding: "0.7rem 1rem", opacity: busy ? 0.6 : 1, cursor: busy ? "not-allowed" : "pointer" }}
      >
        {busy ? "Adding…" : "Add and sync"}
      </button>
      {error && <p style={{ color: theme.red, margin: 0 }}>{error}</p>}
    </div>
  );
}

const styles: Record<string, React.CSSProperties> = {
  overlay: {
    position: "fixed",
    inset: 0,
    background: theme.overlay,
    zIndex: 50,
    display: "flex",
    justifyContent: "flex-end",
  },
  panel: {
    width: "min(440px, 100%)",
    height: "100%",
    background: theme.bgCard,
    borderLeft: `1px solid ${theme.border}`,
    padding: 24,
    overflowY: "auto",
    boxSizing: "border-box",
  },
  head: { display: "flex", alignItems: "flex-start", justifyContent: "space-between", gap: 12, marginBottom: 20 },
  x: { background: "none", border: "none", padding: "0 4px", fontSize: 22, lineHeight: 1.2, color: theme.textDim, cursor: "pointer" },
  field: { display: "block", fontWeight: 700, fontSize: 13, marginBottom: 6 },
};
