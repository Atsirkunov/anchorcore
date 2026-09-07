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

const REST_DEFAULTS: Record<string, string> = {
  base_url: "",
  list_path: "",
  method: "GET",
  auth_mode: "none",
  email: "",
  token: "",
  password: "",
  api_key: "",
  header_name: "X-Api-Key",
  items_path: "",
  map_id: "/id",
  map_title: "/title",
  map_text: "/body",
  map_author: "/author",
  map_updated: "/updated_at",
  page_mode: "none",
  page_param: "",
  next_token_path: "/nextPageToken",
  page_size_param: "",
  page_size: "50",
  page_start: "1",
  offset_param: "",
  limit_param: "",
  max_pages: "20",
  since_param: "",
  body_json: "",
  ref_prefix: "rest",
};

/** Generic REST API mapping form (B44): endpoint + auth, JSON-pointer field map, pagination, live preview. */
function RestFields({
  value,
  onChange,
}: {
  value: Record<string, string>;
  onChange: (next: Record<string, string>) => void;
}) {
  const [preview, setPreview] = useState<null | { count: number; truncated: boolean; docs: { external_id: string; title: string; text_preview: string; author: string; source_ref: string }[] }>(null);
  const [loading, setLoading] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const set = (k: string) => (e: React.ChangeEvent<HTMLInputElement | HTMLSelectElement>) => onChange({ ...value, [k]: e.target.value });
  const v = (k: string) => value[k] ?? REST_DEFAULTS[k] ?? "";

  async function runPreview() {
    setLoading(true);
    setErr(null);
    setPreview(null);
    try {
      if (value.token === "***set***" || value.api_key === "***set***" || value.password === "***set***") {
        throw new Error("Re-enter the credential to preview (the stored secret is hidden)");
      }
      setPreview(await api.previewRest(value));
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }

  const input = (k: string, placeholder: string, type = "text") => (
    <input style={commonStyles.input} type={type} placeholder={placeholder} value={v(k)} onChange={set(k)} />
  );

  return (
    <div style={{ display: "grid", gap: 6 }}>
      <div style={{ fontSize: 12, fontWeight: 700, color: theme.textDim }}>Endpoint</div>
      {input("base_url", "Base URL, e.g. https://api.example.com")}
      {input("list_path", "List path, e.g. /v1/tickets (full http URL also works)")}
      <div style={{ display: "flex", gap: 8 }}>
        <select value={v("method")} onChange={set("method")} style={commonStyles.input} aria-label="HTTP method">
          <option value="GET">GET</option>
          <option value="POST">POST</option>
        </select>
        <select value={v("auth_mode")} onChange={set("auth_mode")} style={commonStyles.input} aria-label="Auth mode">
          <option value="none">No auth</option>
          <option value="bearer">Bearer token</option>
          <option value="basic">Basic (email + password)</option>
          <option value="header">Custom header</option>
        </select>
      </div>
      {v("auth_mode") === "bearer" && input("token", "Bearer token (stored in OS keychain)", "password")}
      {v("auth_mode") === "basic" && (
        <>
          {input("email", "Email / username")}
          {input("password", "Password (stored in OS keychain)", "password")}
        </>
      )}
      {v("auth_mode") === "header" && (
        <>
          {input("header_name", "Header name, e.g. X-Api-Key")}
          {input("api_key", "Header value (stored in OS keychain)", "password")}
        </>
      )}
      <div style={{ fontSize: 12, fontWeight: 700, color: theme.textDim, marginTop: 4 }}>Items & field map (JSON pointers: /fields/summary or fields.summary)</div>
      {input("items_path", "Items pointer, e.g. /data (empty = auto-detect)")}
      {input("map_id", "ID pointer (default /id)")}
      {input("map_title", "Title pointer (default /title)")}
      {input("map_text", "Text pointers, comma-separated (default /body)")}
      {input("map_author", "Author pointer (default /author)")}
      {input("map_updated", "Updated pointer, RFC3339 or epoch (default /updated_at)")}
      <div style={{ fontSize: 12, fontWeight: 700, color: theme.textDim, marginTop: 4 }}>Pagination & incremental</div>
      <div style={{ display: "flex", gap: 8 }}>
        <select value={v("page_mode")} onChange={set("page_mode")} style={commonStyles.input} aria-label="Pagination mode">
          <option value="none">No pagination</option>
          <option value="token">Next-page token</option>
          <option value="page">Page number</option>
          <option value="offset">Offset / limit</option>
        </select>
        {input("page_size", "Page size (default 50)")}
        {input("max_pages", "Max pages (default 20)")}
      </div>
      {v("page_mode") === "token" && (
        <>
          {input("page_param", "Token request param (default pageToken)")}
          {input("next_token_path", "Next-token pointer (default /nextPageToken)")}
        </>
      )}
      {v("page_mode") === "page" && (
        <>
          {input("page_param", "Page param (default page)")}
          {input("page_size_param", "Size param (default per_page)")}
          {input("page_start", "First page (default 1)")}
        </>
      )}
      {v("page_mode") === "offset" && (
        <>
          {input("offset_param", "Offset param (default offset)")}
          {input("limit_param", "Limit param (default limit)")}
        </>
      )}
      {input("since_param", "Incremental param, e.g. updated_since (optional)")}
      {v("method") === "POST" && input("body_json", 'Extra POST body JSON, e.g. {"filter":"open"}')}
      {input("ref_prefix", "Citation prefix (default rest)")}
      <div style={{ display: "flex", gap: 8, alignItems: "center", marginTop: 4 }}>
        <button onClick={runPreview} disabled={loading || !v("base_url") || !v("list_path")} style={commonStyles.button} type="button">
          {loading ? "Previewing…" : "Preview mapping"}
        </button>
        <span style={{ fontSize: 12, color: theme.textDim }}>
          {preview ? `${preview.count} docs${preview.truncated ? " (first 5 shown)" : ""}` : "fetches one page, saves nothing"}
        </span>
      </div>
      {err && <div style={{ fontSize: 12, color: theme.red }}>{err}</div>}
      {preview && (
        <div style={{ display: "grid", gap: 4, borderTop: `1px solid ${theme.border}`, paddingTop: 6 }}>
          {preview.docs.map((d) => (
            <div key={d.source_ref} style={{ fontSize: 12, border: `1px solid ${theme.border}`, borderRadius: 6, padding: 6, background: theme.bgCard }}>
              <div style={{ fontWeight: 600 }}>{d.title || "(no title)"}</div>
              <div style={{ color: theme.textDim }}>{d.source_ref}{d.author ? ` · ${d.author}` : ""}</div>
              <div style={{ color: theme.textMuted }}>{d.text_preview.slice(0, 160)}{d.text_preview.length > 160 ? "…" : ""}</div>
            </div>
          ))}
          {preview.docs.length === 0 && <div style={{ fontSize: 12, color: theme.textDim }}>No docs — check items pointer and field map.</div>}
        </div>
      )}
    </div>
  );
}

/** B37: the "add source" form (folder, Jira, Linear, Drive or generic REST), extracted from SourcesTab. */
export function SourceForm({ onAdded }: { onAdded: () => void }) {
  const [connector, setConnector] = useState("folder");
  const [name, setName] = useState("");
  const [path, setPath] = useState("");
  const [label, setLabel] = useState("internal");
  const [jira, setJira] = useState({ base_url: "", email: "", token: "", project: "" });
  const [gdrive, setGdrive] = useState({ folder_id: "", token: "" });
  const [linear, setLinear] = useState({ api_key: "", team: "" });
  const [rest, setRest] = useState<Record<string, string>>({ ...REST_DEFAULTS });
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
              : connector === "rest"
                ? { ...rest }
                : { base_url: jira.base_url, email: jira.email, token: jira.token, project: jira.project };
      await api.createSource({ connector, name, config, label });
      setName("");
      setPath("");
      setJira({ base_url: "", email: "", token: "", project: "" });
      setGdrive({ folder_id: "", token: "" });
      setLinear({ api_key: "", team: "" });
      setRest({ ...REST_DEFAULTS });
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
          <option value="rest">Generic REST API</option>
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
        ) : connector === "rest" ? (
          <RestFields value={rest} onChange={setRest} />
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
  const [editRest, setEditRest] = useState<Record<string, string>>({ ...REST_DEFAULTS, ...config });
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
      } else if (source.connector === "rest") {
        payload.config = stripPlaceholders({ ...editRest }) as Record<string, string>;
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
      ) : source.connector === "rest" ? (
        <RestFields value={editRest} onChange={setEditRest} />
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
        <button onClick={saveEdit} style={{ ...commonStyles.button, background: theme.accent, color: theme.onAccent }}>
          Save
        </button>
        <button onClick={onCancel} style={commonStyles.button}>
          Cancel
        </button>
      </div>
    </div>
  );
}
