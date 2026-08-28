import { useState } from "react";
import { api } from "../../api";
import { stripPlaceholders } from "../../placeholders";
import { theme, commonStyles } from "../../theme";
import type { Source } from "../../types";

const LABEL_OPTIONS = [
  { value: "internal", label: "Internal" },
  { value: "public", label: "Public" },
  { value: "sensitive", label: "Sensitive" },
  { value: "pii", label: "PII — never sent to cloud providers" },
] as const;

function LabelSelect({
  value,
  onChange,
}: {
  value: string;
  onChange: (v: string) => void;
}) {
  return (
    <select value={value} onChange={(e) => onChange(e.target.value)} style={commonStyles.input}>
      {LABEL_OPTIONS.map((o) => (
        <option key={o.value} value={o.value}>
          {o.label}
        </option>
      ))}
    </select>
  );
}

function parseProjectKeys(project: string): Set<string> {
  return new Set(project.split(",").map((p) => p.trim()).filter(Boolean));
}

function JiraProjectPicker({
  base_url,
  email,
  token,
  project,
  onChange,
}: {
  base_url: string;
  email: string;
  token: string;
  project: string;
  onChange: (next: string) => void;
}) {
  const [projects, setProjects] = useState<{ key: string; name: string }[] | null>(null);
  const [loading, setLoading] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const selected = parseProjectKeys(project);

  async function load() {
    setLoading(true);
    setErr(null);
    try {
      if (token === "***set***") {
        throw new Error("Re-enter API token to load projects (stored token is hidden)");
      }
      const res = await api.listJiraProjects({ base_url, email, token });
      setProjects(res);
      if (res.length === 0) setErr("No projects found or no permission");
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }

  function toggle(key: string) {
    const next = new Set(selected);
    if (next.has(key)) next.delete(key);
    else next.add(key);
    onChange(Array.from(next).join(","));
  }

  return (
    <div style={{ display: "grid", gap: 6, border: `1px solid ${theme.border}`, borderRadius: 6, padding: 8, background: theme.bgCard }}>
      <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
        <button onClick={load} disabled={loading || !base_url || !email || !token} style={commonStyles.button} type="button">
          {loading ? "Loading…" : "Load Jira projects"}
        </button>
        <span style={{ fontSize: 12, color: theme.textDim }}>
          {projects ? `${projects.length} projects` : "requires base URL, email & token"}
        </span>
      </div>
      {err && <div style={{ fontSize: 12, color: theme.red }}>{err}</div>}
      {projects && (
        <div style={{ maxHeight: 160, overflowY: "auto", display: "grid", gap: 4, borderTop: `1px solid ${theme.border}`, paddingTop: 6 }}>
          {projects.map((p) => (
            <label key={p.key} style={{ display: "flex", gap: 6, alignItems: "center", fontSize: 13, cursor: "pointer" }}>
              <input type="checkbox" checked={selected.has(p.key)} onChange={() => toggle(p.key)} />
              <span style={{ fontWeight: 600 }}>{p.key}</span>
              <span style={{ color: theme.textDim }}>{p.name}</span>
            </label>
          ))}
        </div>
      )}
      <div style={{ fontSize: 11, color: theme.textMuted }}>Selected: {project || "(none — pick above or type manually)"}</div>
    </div>
  );
}

/** B37: the "add source" form (folder, Jira, Linear or Drive), extracted from SourcesTab. */
export function SourceForm({ onAdded }: { onAdded: () => void }) {
  const [connector, setConnector] = useState("folder");
  const [name, setName] = useState("");
  const [path, setPath] = useState("");
  const [label, setLabel] = useState("internal");
  const [jira, setJira] = useState({ base_url: "", email: "", token: "", project: "" });
  const [gdrive, setGdrive] = useState({ folder_id: "", token: "" });
  const [linear, setLinear] = useState({ api_key: "", team: "" });
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function addSource() {
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
              : { base_url: jira.base_url, email: jira.email, token: jira.token, project: jira.project };
      await api.createSource({ connector, name, config, label });
      setName("");
      setPath("");
      setJira({ base_url: "", email: "", token: "", project: "" });
      setGdrive({ folder_id: "", token: "" });
      setLinear({ api_key: "", team: "" });
      setLabel("internal");
      onAdded();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <>
      <div style={{ display: "grid", gap: 8, marginBottom: 16 }}>
        <select value={connector} onChange={(e) => setConnector(e.target.value)} style={commonStyles.input}>
          <option value="folder">Local folder</option>
          <option value="jira">Jira</option>
          <option value="linear">Linear</option>
          <option value="gdrive">Google Drive</option>
        </select>
        <input style={commonStyles.input} placeholder="Source name" value={name} onChange={(e) => setName(e.target.value)} />
        {connector === "folder" ? (
          <input style={commonStyles.input} placeholder="Folder path (absolute)" value={path} onChange={(e) => setPath(e.target.value)} />
        ) : connector === "gdrive" ? (
          <>
            <input style={commonStyles.input} placeholder="Drive folder ID (from URL …/folders/…)" value={gdrive.folder_id} onChange={(e) => setGdrive({ ...gdrive, folder_id: e.target.value })} />
            <input style={commonStyles.input} type="password" placeholder="OAuth access token (Bearer)" value={gdrive.token} onChange={(e) => setGdrive({ ...gdrive, token: e.target.value })} />
            <div style={{ fontSize: 12, color: theme.textDim }}>Needs a Google Cloud OAuth token — see docs/sample-dataset.md or hosting/README for setup. Token is stored in OS keychain.</div>
          </>
        ) : connector === "linear" ? (
          <>
            <input style={commonStyles.input} type="password" placeholder="Linear API key (lin_api_...)" value={linear.api_key} onChange={(e) => setLinear({ ...linear, api_key: e.target.value })} />
            <input style={commonStyles.input} placeholder="Team key, e.g. ENG or ENG,PM" value={linear.team} onChange={(e) => setLinear({ ...linear, team: e.target.value })} />
            <div style={{ fontSize: 12, color: theme.textDim }}>Find API key at linear.app/settings/api · Team key is the 2–3 letter prefix (e.g. ENG). Token stored in OS keychain.</div>
          </>
        ) : (
          <>
            <input style={commonStyles.input} placeholder="Jira base URL, e.g. https://x.atlassian.net" value={jira.base_url} onChange={(e) => setJira({ ...jira, base_url: e.target.value })} />
            <input style={commonStyles.input} placeholder="Email" value={jira.email} onChange={(e) => setJira({ ...jira, email: e.target.value })} />
            <input style={commonStyles.input} type="password" placeholder="API token" value={jira.token} onChange={(e) => setJira({ ...jira, token: e.target.value })} />
            <input style={commonStyles.input} placeholder="Project keys, e.g. PM or PM,TEST (or pick below)" value={jira.project} onChange={(e) => setJira({ ...jira, project: e.target.value })} />
            <JiraProjectPicker base_url={jira.base_url} email={jira.email} token={jira.token} project={jira.project} onChange={(v) => setJira({ ...jira, project: v })} />
          </>
        )}
        <LabelSelect value={label} onChange={setLabel} />
        <button onClick={addSource} disabled={busy} style={commonStyles.button}>
          {busy ? "Adding…" : "Add source"}
        </button>
      </div>
      {error && <p style={{ color: theme.red }}>{error}</p>}
    </>
  );
}

/** B37: inline edit form for one source (B7), extracted from SourcesTab. */
export function SourceEditForm({
  source,
  config,
  onSaved,
  onCancel,
}: {
  source: Source;
  config: Record<string, string>;
  onSaved: () => void;
  onCancel: () => void;
}) {
  const [editName, setEditName] = useState(source.name);
  const [editEnabled, setEditEnabled] = useState(source.enabled);
  const [editLabel, setEditLabel] = useState<string>(source.label);
  const [editPath, setEditPath] = useState(config.path ?? "");
  const [editJira, setEditJira] = useState({
    base_url: config.base_url ?? "",
    email: config.email ?? "",
    token: config.token ?? "",
    project: config.project ?? "",
  });
  const [editGdrive, setEditGdrive] = useState({
    folder_id: config.folder_id ?? "",
    token: config.token ?? "",
  });
  const [editLinear, setEditLinear] = useState({
    api_key: config.api_key ?? config.token ?? "",
    team: config.team ?? config.project ?? "",
  });
  const [error, setError] = useState<string | null>(null);

  async function saveEdit() {
    setError(null);
    try {
      const payload: { name: string; enabled: boolean; label: string; config?: Record<string, string> } = {
        name: editName.trim(),
        enabled: editEnabled,
        label: editLabel,
      };
      if (source.connector === "folder") {
        payload.config = { path: editPath };
      } else if (source.connector === "gdrive") {
        payload.config = stripPlaceholders({ folder_id: editGdrive.folder_id, token: editGdrive.token }) as Record<string, string>;
      } else if (source.connector === "linear") {
        payload.config = stripPlaceholders({ api_key: editLinear.api_key, team: editLinear.team }) as Record<string, string>;
      } else {
        // strip the "***set***" token placeholder so it never overwrites the
        // stored secret (B36: shared helper with SettingsTab)
        payload.config = stripPlaceholders({ ...editJira }) as Record<string, string>;
      }
      await api.updateSource(source.id, payload);
      onSaved();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  return (
    <div style={{ display: "grid", gap: 6, marginTop: 10, paddingTop: 10, borderTop: `1px solid ${theme.border}` }}>
      <input style={commonStyles.input} placeholder="Source name" value={editName} onChange={(e) => setEditName(e.target.value)} />
      {source.connector === "folder" ? (
        <input style={commonStyles.input} placeholder="Folder path (absolute)" value={editPath} onChange={(e) => setEditPath(e.target.value)} />
      ) : source.connector === "gdrive" ? (
        <>
          <input style={commonStyles.input} placeholder="Drive folder ID" value={editGdrive.folder_id} onChange={(e) => setEditGdrive({ ...editGdrive, folder_id: e.target.value })} />
          <input
            style={commonStyles.input}
            type="password"
            placeholder={editGdrive.token === "***set***" ? "•••••••• (stored — type to replace)" : "OAuth token"}
            value={editGdrive.token}
            onChange={(e) => setEditGdrive({ ...editGdrive, token: e.target.value })}
          />
        </>
      ) : source.connector === "linear" ? (
        <>
          <input
            style={commonStyles.input}
            type="password"
            placeholder={editLinear.api_key === "***set***" ? "•••••••• (stored — type to replace)" : "Linear API key"}
            value={editLinear.api_key}
            onChange={(e) => setEditLinear({ ...editLinear, api_key: e.target.value })}
          />
          <input style={commonStyles.input} placeholder="Team key, e.g. ENG or ENG,PM" value={editLinear.team} onChange={(e) => setEditLinear({ ...editLinear, team: e.target.value })} />
        </>
      ) : (
        <>
          <input style={commonStyles.input} placeholder="Jira base URL" value={editJira.base_url} onChange={(e) => setEditJira({ ...editJira, base_url: e.target.value })} />
          <input style={commonStyles.input} placeholder="Email" value={editJira.email} onChange={(e) => setEditJira({ ...editJira, email: e.target.value })} />
          <input
            style={commonStyles.input}
            type="password"
            placeholder={editJira.token === "***set***" ? "•••••••• (stored — type to replace)" : "API token"}
            value={editJira.token}
            onChange={(e) => setEditJira({ ...editJira, token: e.target.value })}
          />
          <input style={commonStyles.input} placeholder="Project keys, e.g. PM or PM,TEST" value={editJira.project} onChange={(e) => setEditJira({ ...editJira, project: e.target.value })} />
          <JiraProjectPicker base_url={editJira.base_url} email={editJira.email} token={editJira.token} project={editJira.project} onChange={(v) => setEditJira({ ...editJira, project: v })} />
        </>
      )}
      <LabelSelect value={editLabel} onChange={setEditLabel} />
      <label style={{ display: "flex", alignItems: "center", gap: 6, fontSize: 13, color: theme.textMuted }}>
        <input type="checkbox" checked={editEnabled} onChange={(e) => setEditEnabled(e.target.checked)} />
        Enabled (syncs and watchers respect this)
      </label>
      {error && <p style={{ color: theme.red }}>{error}</p>}
      <div style={{ display: "flex", gap: 8 }}>
        <button onClick={saveEdit} style={{ ...commonStyles.button, background: theme.accent, color: "#fff" }}>
          Save
        </button>
        <button onClick={onCancel} style={commonStyles.button}>
          Cancel
        </button>
      </div>
    </div>
  );
}
