import { useState } from "react";
import { api } from "../../api";
import { theme, commonStyles } from "../../theme";
import type { Job, Source } from "../../types";
import { SourceEditForm } from "./SourceForm";

function jobLabel(job: Job): string {
  const verb = job.kind === "reclassify" ? "Reclassifying" : "Syncing";
  if (job.total > 0) return `${verb}… ${job.processed}/${job.total}`;
  return `${verb}…`;
}

const LABEL_ALLOWS: Record<string, string> = {
  public: "answerable in shared/public scope",
  sensitive: "local models only · never shared",
  pii: "local models only · never shared",
};

/** B37: one source row — status, running job badge, actions, inline edit form. */
export function SourceRow({
  source,
  config,
  job,
  onChanged,
}: {
  source: Source;
  config: Record<string, string>;
  job: Job | undefined;
  onChanged: () => void;
}) {
  const [editing, setEditing] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);

  function configSummary(): string {
    if (source.connector === "folder") return config.path ? `📁 ${config.path}` : "folder source";
    if (source.connector === "gdrive") return config.folder_id ? `📂 Drive:${config.folder_id.slice(0, 10)}…` : "Drive source";
    if (source.connector === "rest") return config.base_url ? `🔌 ${(config.base_url + (config.list_path ?? "")).slice(0, 48)}` : "REST source";
    const parts = [config.base_url, config.project].filter(Boolean);
    return parts.length ? `🔗 ${parts.join(" · ")}` : "Jira source";
  }

  async function startJob(id: number, kind: "sync" | "reclassify") {
    try {
      setActionError(null);
      if (kind === "sync") await api.syncSource(id);
      else await api.reclassifySource(id);
      onChanged();
    } catch (e) {
      setActionError(e instanceof Error ? e.message : String(e));
    }
  }

  async function deleteSource(id: number, name: string) {
    if (!window.confirm(`Delete source "${name}"? Its synced items, entities and jobs are removed for good. This cannot be undone.`)) return;
    try {
      setActionError(null);
      await api.deleteSource(id);
      onChanged();
    } catch (e) {
      setActionError(e instanceof Error ? e.message : String(e));
    }
  }

  async function cancelJob(job: Job) {
    try {
      await api.cancelJob(job.id);
      onChanged();
    } catch (e) {
      console.error(e);
    }
  }

  return (
    <li
      style={{
        background: theme.bgCard,
        border: `1px solid ${theme.border}`,
        borderRadius: 8,
        padding: "0.6rem 0.9rem",
      }}
    >
      <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", gap: 12 }}>
        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{ fontWeight: 600 }}>
            {source.name}
            {source.error_count > 0 && (
              <span
                style={{
                  marginLeft: 8,
                  fontSize: 11,
                  background: theme.redBg,
                  color: theme.redText,
                  padding: "0.1rem 0.5rem",
                  borderRadius: 999,
                  fontWeight: 700,
                }}
              >
                sync failing ×{source.error_count}
              </span>
            )}
          </div>
        <div style={{ fontSize: 12, color: theme.textMuted }}>
          {source.connector} · last sync: {source.last_synced_at ? new Date(source.last_synced_at).toLocaleString() : "never"}
        </div>
        <div style={{ fontSize: 12, color: theme.textMuted, marginTop: 2 }} title={configSummary()}>
          {configSummary()}
        </div>
        {source.label !== "internal" && (
          <div style={{ fontSize: 11, color: source.label === "pii" ? theme.red : source.label === "sensitive" ? theme.amber : theme.textMuted, marginTop: 2 }}>
            label: {source.label} — {LABEL_ALLOWS[source.label]}
          </div>
        )}
          {job && (
            <div style={{ marginTop: 6 }}>
              <div style={{ fontSize: 12, color: theme.accentAlt }}>{jobLabel(job)}</div>
              {job.total > 0 && (
                <div style={{ height: 6, borderRadius: 3, background: theme.border, marginTop: 4, overflow: "hidden" }}>
                  <div
                    style={{
                      height: "100%",
                      width: `${Math.min(100, (job.processed / job.total) * 100)}%`,
                      background: theme.blue,
                      transition: "width 0.3s",
                    }}
                  />
                </div>
              )}
            </div>
          )}
          {source.last_error && (
            <div style={{ fontSize: 12, color: theme.red, marginTop: 2 }} title={source.last_error}>
              {source.last_error.slice(0, 120)}
            </div>
          )}
          {actionError && (
            <div style={{ fontSize: 12, color: theme.red, marginTop: 4 }} title={actionError}>
              Action failed: {actionError.slice(0, 200)}
            </div>
          )}
        </div>
        <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
          <button onClick={() => setEditing((v) => !v)} style={commonStyles.button}>
            {editing ? "Editing…" : "Edit"}
          </button>
          <button onClick={() => startJob(source.id, "sync")} disabled={!!job} style={commonStyles.button}>
            {job?.kind === "sync" ? "Syncing…" : "Sync now"}
          </button>
          <button onClick={() => startJob(source.id, "reclassify")} disabled={!!job} style={commonStyles.button}>
            {job?.kind === "reclassify" ? "Reclassifying…" : "Reclassify"}
          </button>
          {job && (
            <button
              onClick={() => cancelJob(job)}
              style={{ ...commonStyles.button, background: theme.redBg, color: theme.redText }}
              title="Stop this job"
            >
              Stop
            </button>
          )}
          <button
            onClick={() => deleteSource(source.id, source.name)}
            disabled={!!job}
            title={job ? "Stop the running job first" : "Delete this source and its synced data"}
            style={{ ...commonStyles.button, background: theme.redBg }}
          >
            Delete
          </button>
        </div>
      </div>
      {editing && (
        <SourceEditForm
          source={source}
          config={config}
          onSaved={() => {
            setEditing(false);
            onChanged();
          }}
          onCancel={() => setEditing(false)}
        />
      )}
    </li>
  );
}
