import { useState } from "react";
import { api, errMsg } from "../../api";
import { FolderPicker } from "../../FolderPicker";
import { LABEL_OPTIONS } from "./labels";
import { stripPlaceholders } from "../../placeholders";
import { theme, commonStyles, radius } from "../../theme";
import type { Source } from "../../types";
import { REST_DEFAULTS } from "./restDefaults";

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

export function JiraProjectPicker({
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
      setErr(errMsg(e));
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
    <div style={{ display: "grid", gap: 6, border: `1px solid ${theme.border}`, borderRadius: radius, padding: 8, background: theme.bgCard }}>
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
      <div style={{ fontSize: 11, color: theme.textMuted }}>Selected: {project || "(none: pick above or type one)"}</div>
    </div>
  );
}

/** Generic REST API mapping form (B44): endpoint + auth, JSON-pointer field map, pagination, live preview. */
export function RestFields({
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
      setErr(errMsg(e));
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
            <div key={d.source_ref} style={{ fontSize: 12, border: `1px solid ${theme.border}`, borderRadius: radius, padding: 6, background: theme.bgCard }}>
              <div style={{ fontWeight: 600 }}>{d.title || "(no title)"}</div>
              <div style={{ color: theme.textDim }}>{d.source_ref}{d.author ? ` · ${d.author}` : ""}</div>
              <div style={{ color: theme.textMuted }}>{d.text_preview.slice(0, 160)}{d.text_preview.length > 160 ? "…" : ""}</div>
            </div>
          ))}
          {preview.docs.length === 0 && <div style={{ fontSize: 12, color: theme.textDim }}>No docs. Check the items pointer and field map.</div>}
        </div>
      )}
    </div>
  );
}

/** B37: inline edit form for one source (B7), extracted from SourcesTab (now LibraryTab). */
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
  const [editPickerOpen, setEditPickerOpen] = useState(false);
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
  const [editMiro, setEditMiro] = useState({
    token: config.token ?? "",
    boards: config.boards ?? "",
    team_id: config.team_id ?? "",
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
      } else if (source.connector === "miro") {
        payload.config = stripPlaceholders({ ...editMiro }) as Record<string, string>;
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
      setError(errMsg(e));
    }
  }

  return (
    <div style={{ display: "grid", gap: 6, marginTop: 10, paddingTop: 10, borderTop: `1px solid ${theme.border}` }}>
      <input style={commonStyles.input} placeholder="Source name" value={editName} onChange={(e) => setEditName(e.target.value)} />
      {source.connector === "folder" ? (
        <>
          <div style={{ display: "flex", gap: 8 }}>
            <input style={{ ...commonStyles.input, flex: 1 }} placeholder="Folder path (absolute)" value={editPath} onChange={(e) => setEditPath(e.target.value)} />
            <button type="button" style={commonStyles.button} title="Browse folders on this machine" onClick={() => setEditPickerOpen((v) => !v)}>
              Browse…
            </button>
          </div>
          {editPickerOpen && (
            <FolderPicker
              initialPath={editPath}
              onSelect={(p) => {
                setEditPath(p);
                setEditPickerOpen(false);
              }}
              onCancel={() => setEditPickerOpen(false)}
            />
          )}
        </>
      ) : source.connector === "gdrive" ? (
        <>
          <input style={commonStyles.input} placeholder="Drive folder ID" value={editGdrive.folder_id} onChange={(e) => setEditGdrive({ ...editGdrive, folder_id: e.target.value })} />
          <input
            style={commonStyles.input}
            type="password"
            placeholder={editGdrive.token === "***set***" ? "•••••••• (stored; type to replace)" : "OAuth token"}
            value={editGdrive.token}
            onChange={(e) => setEditGdrive({ ...editGdrive, token: e.target.value })}
          />
        </>
      ) : source.connector === "linear" ? (
        <>
          <input
            style={commonStyles.input}
            type="password"
            placeholder={editLinear.api_key === "***set***" ? "•••••••• (stored; type to replace)" : "Linear API key"}
            value={editLinear.api_key}
            onChange={(e) => setEditLinear({ ...editLinear, api_key: e.target.value })}
          />
          <input style={commonStyles.input} placeholder="Team key, e.g. ENG or ENG,PM" value={editLinear.team} onChange={(e) => setEditLinear({ ...editLinear, team: e.target.value })} />
        </>
      ) : source.connector === "miro" ? (
        <>
          <input
            style={commonStyles.input}
            type="password"
            placeholder={editMiro.token === "***set***" ? "•••••••• (stored; type to replace)" : "Miro access token"}
            value={editMiro.token}
            onChange={(e) => setEditMiro({ ...editMiro, token: e.target.value })}
          />
          <input style={commonStyles.input} placeholder="Board IDs, comma-separated" value={editMiro.boards} onChange={(e) => setEditMiro({ ...editMiro, boards: e.target.value })} />
          <input style={commonStyles.input} placeholder="Team ID (optional)" value={editMiro.team_id} onChange={(e) => setEditMiro({ ...editMiro, team_id: e.target.value })} />
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
            placeholder={editJira.token === "***set***" ? "•••••••• (stored; type to replace)" : "API token"}
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
