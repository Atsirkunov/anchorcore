import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { api } from "../../api";
import { theme, commonStyles, radius } from "../../theme";
import type { Job, Source } from "../../types";
import { JobStopButton, StopNoteLine } from "../../JobStopButton";
import { useJobStop } from "../../useJobStop";
import { SourceEditForm } from "./SourceForm";

function jobLabel(job: Job): string {
  if (job.status === "pending") return `Queued ${job.kind} #${job.id}`;
  const verb = job.kind === "reclassify" ? "Reclassifying" : job.kind === "embed" ? "Embedding" : "Syncing";
  if (job.total > 0) {
    // `processed/total` counts files scanned; `result.items` is the live
    // changed count, so "514/514 · 3 changed" can't read as a full re-sync.
    const changed = job.result?.items;
    const suffix = typeof changed === "number" ? ` · ${changed} changed` : "";
    return `${verb} #${job.id}… ${job.processed}/${job.total}${suffix}`;
  }
  return `${verb} #${job.id}…`;
}

const LABEL_ALLOWS: Record<string, string> = {
  public: "answerable in shared/public scope",
  sensitive: "local models only · never shared",
  // "pii" deliberately absent (deprecated 2026-10-13): legacy pii sources fall
  // back to the sensitive hint below instead of rendering "undefined".
};

/** B37: one source row — status, every active job with its own Stop
 * control (R30.4), actions, inline edit form. */
export function SourceRow({
  source,
  config,
  jobs,
  onChanged,
}: {
  source: Source;
  config: Record<string, string>;
  /** All pending/running jobs of this source, oldest first. */
  jobs: Job[];
  onChanged: () => void;
}) {
  const [editing, setEditing] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  // R30.1: disable both triggers while the POST is in flight — the jobs poll
  // lags the click by up to one interval, so without this a second click
  // slips through (the backend 409s it, but the button should never be live).
  const [busy, setBusy] = useState(false);
  const queryClient = useQueryClient();
  const stopper = useJobStop(onChanged);
  // R30.1: sync + reclassify share one slot per source; embed jobs don't block them.
  const syncJob = jobs.find((j) => j.kind === "sync" || j.kind === "reclassify");

  function configSummary(): string {
    if (source.connector === "folder") return config.path ? config.path : "folder source";
    if (source.connector === "gdrive") return config.folder_id ? `Drive: ${config.folder_id.slice(0, 10)}…` : "Drive source";
    if (source.connector === "rest") return config.base_url ? (config.base_url + (config.list_path ?? "")).slice(0, 48) : "REST source";
    const parts = [config.base_url, config.project].filter(Boolean);
    return parts.length ? parts.join(" · ") : "Jira source";
  }

  async function startJob(id: number, kind: "sync" | "reclassify") {
    if (busy || syncJob) return;
    try {
      setBusy(true);
      setActionError(null);
      if (kind === "sync") await api.syncSource(id);
      else await api.reclassifySource(id);
      onChanged();
      // Refetch jobs immediately instead of waiting for the next poll tick,
      // so the row flips to Syncing… before a second click can land.
      await queryClient.invalidateQueries({ queryKey: ["jobs"] });
    } catch (e) {
      // R30.1: a 409 here names the live job ("…already active…(job N)").
      setActionError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
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

  return (
    <li
      style={{
        background: theme.bgCard,
        border: `1px solid ${theme.border}`,
        borderRadius: radius,
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
                  borderRadius: radius,
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
        <div style={{ fontSize: 12, fontFamily: theme.mono, color: theme.textMuted, marginTop: 2, overflowWrap: "anywhere" }} title={configSummary()}>
          {configSummary()}
        </div>
        {source.label !== "internal" && (
          <div style={{ fontSize: 11, color: source.label === "sensitive" || source.label === "pii" ? theme.amber : theme.textMuted, marginTop: 2 }}>
            label: {source.label === "pii" ? "pii (deprecated — behaves like sensitive)" : source.label} — {LABEL_ALLOWS[source.label] ?? LABEL_ALLOWS.sensitive}
          </div>
        )}
          {jobs.map((job) => (
            <div key={job.id} style={{ marginTop: 6 }}>
              <div style={{ display: "flex", alignItems: "center", gap: 8, flexWrap: "wrap" }}>
                <span style={{ fontSize: 12, color: theme.accentAlt }}>{jobLabel(job)}</span>
                <JobStopButton job={job} sourceName={source.name} busy={stopper.inFlight.has(job.id)} onStop={stopper.stop} />
              </div>
              {job.total > 0 && (
                <div style={{ height: 6, borderRadius: radius, background: theme.border, marginTop: 4, overflow: "hidden" }}>
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
          ))}
          {Object.entries(stopper.notes).map(([id, note]) => (
            <StopNoteLine key={id} note={note} />
          ))}
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
          <button onClick={() => startJob(source.id, "sync")} disabled={!!syncJob || busy} style={commonStyles.button}>
            {syncJob?.kind === "sync" ? "Syncing…" : "Sync now"}
          </button>
          <button onClick={() => startJob(source.id, "reclassify")} disabled={!!syncJob || busy} style={commonStyles.button}>
            {syncJob?.kind === "reclassify" ? "Reclassifying…" : "Reclassify"}
          </button>
          <button
            onClick={() => deleteSource(source.id, source.name)}
            disabled={jobs.length > 0}
            title={jobs.length > 0 ? "Stop the active jobs first" : "Delete this source and its synced data"}
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
