import { useEffect, useRef, useState } from "react";
import { api } from "./api";
import { theme, radius } from "./theme";
import type { BackupImportReport, Job } from "./types";

/**
 * R25.5 — "Backup & restore" card (Phase A of docs/plan-backup-sync.md).
 * Export a portable .anchorcore-backup archive; import one with a dry-run
 * report first, then merge or replace with a typed confirmation.
 */

function CountsLine({ title, counts }: { title: string; counts: Record<string, number> }) {
  const entries = Object.entries(counts).filter(([, v]) => v > 0);
  if (entries.length === 0) return null;
  return (
    <div style={{ fontSize: 12, color: theme.textMuted }}>
      {title}: {entries.map(([k, v]) => `${k} ${v}`).join(" · ")}
    </div>
  );
}

function shortUid(uid: string): string {
  return uid.length > 12 ? `${uid.slice(0, 8)}…` : uid;
}

/** Polls the queued re-embed jobs so a long import still shows progress. */
function ReembedProgress({ jobIds }: { jobIds: number[] }) {
  const [jobs, setJobs] = useState<Job[]>([]);

  useEffect(() => {
    if (jobIds.length === 0) return;
    let alive = true;
    const tick = async () => {
      const out: Job[] = [];
      for (const id of jobIds) {
        try {
          out.push(await api.getJob(id));
        } catch {
          // job gone — skip
        }
      }
      if (alive) setJobs(out);
    };
    void tick();
    const timer = setInterval(tick, 2000);
    return () => {
      alive = false;
      clearInterval(timer);
    };
  }, [jobIds]);

  if (jobIds.length === 0) return null;
  const processed = jobs.reduce((a, j) => a + (j.processed || 0), 0);
  const total = jobs.reduce((a, j) => a + (j.total || 0), 0);
  const done = jobs.filter((j) => j.status === "done").length;
  const failed = jobs.some((j) => j.status === "failed");
  return (
    <div style={{ fontSize: 12, marginTop: 6, color: failed ? theme.red : theme.textMuted }}>
      Re-embedding imported chunks: {done}/{jobIds.length} job(s)
      {total > 0 ? ` — ${processed}/${total} chunks` : " — queued"}
      {failed ? " — a job failed (see System → Errors). Answers stay keyword-only for those chunks." : ""}
    </div>
  );
}

export function BackupCard() {
  const [includeSecrets, setIncludeSecrets] = useState(false);
  const [passphrase, setPassphrase] = useState("");
  const [includeEmbeddings, setIncludeEmbeddings] = useState(false);
  const [includeOps, setIncludeOps] = useState(false);
  const [busy, setBusy] = useState<"export" | "import" | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const [file, setFile] = useState<File | null>(null);
  const [report, setReport] = useState<BackupImportReport | null>(null);
  const [remap, setRemap] = useState<Record<string, string>>({});
  const [confirmText, setConfirmText] = useState("");
  const [jobIds, setJobIds] = useState<number[]>([]);
  const fileInput = useRef<HTMLInputElement | null>(null);

  const pickFile = (f: File | null) => {
    setFile(f);
    setReport(null);
    setJobIds([]);
    setRemap({});
    setConfirmText("");
    setNote(null);
    setError(null);
  };

  const doExport = async () => {
    setBusy("export");
    setError(null);
    setNote(null);
    try {
      const { blob, filename } = await api.backupExport({
        include_secrets: includeSecrets,
        passphrase: includeSecrets ? passphrase : undefined,
        include_embeddings: includeEmbeddings,
        include_ops: includeOps,
      });
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = filename;
      document.body.appendChild(a);
      a.click();
      a.remove();
      URL.revokeObjectURL(url);
      setNote(`Exported ${filename} (${Math.max(1, Math.round(blob.size / 1024))} KB) — saved by your browser.`);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  };

  const runImport = async (dryRun: boolean, mode: "merge" | "replace") => {
    if (!file) return;
    setBusy("import");
    setError(null);
    setNote(null);
    try {
      const entries = Object.entries(remap).filter(([, v]) => v.trim() !== "");
      const result = await api.backupImport(file, {
        mode,
        dry_run: dryRun,
        confirm: mode === "replace" && !dryRun ? "replace" : undefined,
        passphrase: passphrase || undefined,
        remap: entries.length > 0 ? Object.fromEntries(entries) : undefined,
      });
      setReport(result);
      setRemap((prev) => {
        const next = { ...prev };
        for (const fp of result.folder_paths) {
          if (!(fp.source_uid in next)) next[fp.source_uid] = fp.path;
        }
        return next;
      });
      if (!dryRun) {
        setJobIds(result.reembed_job_ids ?? []);
        const imported = Object.values(result.imported).reduce((a, b) => a + b, 0);
        const kept = Object.values(result.skipped).reduce((a, b) => a + b, 0);
        setNote(
          `Imported ${imported} record(s)${kept > 0 ? `, kept ${kept} local` : ""}` +
            `${result.secrets_imported > 0 ? `, ${result.secrets_imported} credential(s)` : ""}` +
            `${result.safety_backup ? ` · safety backup: ${result.safety_backup}` : ""}. ` +
            "Reload the app to see the restored sources everywhere."
        );
        setConfirmText("");
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  };

  const secretsReady = !includeSecrets || passphrase.trim().length > 0;
  const replaceReady = file !== null && confirmText.trim().toLowerCase() === "replace" && busy === null;

  return (
    <div style={{ ...styles.card, marginBottom: 16 }}>
      <div style={styles.cardTitle}>Backup &amp; restore</div>
      <div style={{ fontSize: 12, color: theme.textMuted, marginBottom: 8 }}>
        A portable <code>.anchorcore-backup</code> archive carries your memory (sources, documents, entities,
        review state) to another device. Credentials stay out unless you opt in with a passphrase; embeddings are
        recomputed on the new device.
      </div>

      <div style={styles.row}>
        <label style={styles.check}>
          <input type="checkbox" checked={includeSecrets} onChange={(e) => setIncludeSecrets(e.target.checked)} />
          Include source credentials (encrypted)
        </label>
        {includeSecrets && (
          <input
            type="password"
            value={passphrase}
            onChange={(e) => setPassphrase(e.target.value)}
            placeholder="Passphrase (not stored — losing it loses the bundle)"
            style={{ ...styles.input, marginTop: 4 }}
          />
        )}
        <label style={styles.check}>
          <input type="checkbox" checked={includeEmbeddings} onChange={(e) => setIncludeEmbeddings(e.target.checked)} />
          Include embeddings (same model only)
        </label>
        <label style={styles.check}>
          <input type="checkbox" checked={includeOps} onChange={(e) => setIncludeOps(e.target.checked)} />
          Include jobs, events and users
        </label>
        <div>
          <button
            style={styles.button}
            disabled={busy !== null || !secretsReady}
            title={secretsReady ? "Download the archive" : "Enter a passphrase first"}
            onClick={doExport}
          >
            {busy === "export" ? "Exporting…" : "Export archive"}
          </button>
        </div>
      </div>

      <div style={{ ...styles.row, marginTop: 8 }}>
        <div style={{ fontWeight: 600 }}>Import</div>
        <input
          ref={fileInput}
          type="file"
          accept=".anchorcore-backup,application/zip"
          onChange={(e) => pickFile(e.target.files?.[0] ?? null)}
          style={{ fontSize: 12 }}
        />
        {file && (
          <div style={{ display: "flex", gap: 6, flexWrap: "wrap" }}>
            <button style={styles.button} disabled={busy !== null} onClick={() => void runImport(true, "merge")}>
              {busy === "import" ? "Working…" : "Preview (dry run)"}
            </button>
            <button style={styles.primary} disabled={busy !== null} onClick={() => void runImport(false, "merge")}>
              Import (merge)
            </button>
          </div>
        )}
        {report && (
          <div style={{ marginTop: 6 }}>
            <CountsLine title="In archive" counts={report.counts} />
            <CountsLine title={report.dry_run ? "Would import" : "Imported"} counts={report.imported} />
            <CountsLine title="Kept local" counts={report.skipped} />
            {report.conflicts.length > 0 && (
              <div style={{ fontSize: 12, color: theme.amber }}>
                {report.conflicts.length} conflict(s) resolved {report.dry_run ? "(would be)" : ""} — local newer
                state wins.
              </div>
            )}
            {report.needs_secret.length > 0 && (
              <div style={{ fontSize: 12, marginTop: 4 }}>
                <div style={{ color: theme.amber }}>
                  Credentials to re-enter ({report.needs_secret.length}) — Sources tab:
                </div>
                {report.needs_secret.map((s) => (
                  <div key={`${s.source_uid}:${s.field}`} style={{ color: theme.textMuted }}>
                    {s.source_name || shortUid(s.source_uid)} — {s.field}
                  </div>
                ))}
              </div>
            )}
            {report.folder_paths.length > 0 && (
              <div style={{ fontSize: 12, marginTop: 4 }}>
                <div style={{ color: theme.textMuted }}>Folder paths on this machine:</div>
                {report.folder_paths.map((fp) => (
                  <div key={fp.source_uid} style={{ display: "flex", gap: 6, alignItems: "center", marginTop: 2 }}>
                    <span style={{ minWidth: 90, color: theme.textMuted }}>{fp.name}</span>
                    <input
                      value={remap[fp.source_uid] ?? fp.path}
                      onChange={(e) => setRemap((r) => ({ ...r, [fp.source_uid]: e.target.value }))}
                      style={{ ...styles.input, flex: 1, fontSize: 12 }}
                    />
                  </div>
                ))}
              </div>
            )}
            {report.warnings.length > 0 && (
              <div style={{ fontSize: 12, color: theme.amber, marginTop: 4 }}>
                {report.warnings.slice(0, 3).map((w) => (
                  <div key={w}>{w}</div>
                ))}
              </div>
            )}
            {!report.dry_run && <ReembedProgress jobIds={jobIds} />}
          </div>
        )}
        {file && (
          <div style={{ marginTop: 8, borderTop: `1px solid ${theme.borderSoft}`, paddingTop: 6 }}>
            <div style={{ fontSize: 12, color: theme.red }}>
              Replace deletes all local data first (a safety backup is taken automatically). Type{" "}
              <code>replace</code> to enable it.
            </div>
            <div style={{ display: "flex", gap: 6, marginTop: 4 }}>
              <input
                value={confirmText}
                onChange={(e) => setConfirmText(e.target.value)}
                placeholder="replace"
                style={{ ...styles.input, fontSize: 12, width: 140 }}
              />
              <button
                style={{ ...styles.button, borderColor: theme.red, color: theme.redText }}
                disabled={!replaceReady}
                onClick={() => void runImport(false, "replace")}
              >
                Replace all data
              </button>
            </div>
          </div>
        )}
      </div>

      {note && <div style={{ fontSize: 12, color: theme.green, marginTop: 8 }}>{note}</div>}
      {error && (
        <div style={{ fontSize: 12, color: theme.red, marginTop: 8, wordBreak: "break-word" }}>
          {error}
        </div>
      )}
    </div>
  );
}

const styles = {
  card: { background: theme.bgCard, border: `1px solid ${theme.border}`, borderRadius: radius, padding: "0.7rem 0.9rem" },
  cardTitle: { fontWeight: 600, marginBottom: 4 },
  row: {
    display: "flex",
    flexDirection: "column" as const,
    gap: 4,
    background: theme.bgCard,
    border: `1px solid ${theme.border}`,
    borderRadius: radius,
    padding: "0.5rem 0.75rem",
  },
  check: { display: "flex", gap: 6, alignItems: "center", fontSize: 13 },
  input: {
    padding: "0.4rem 0.6rem",
    borderRadius: radius,
    border: `1px solid ${theme.border}`,
    background: theme.bg,
    color: theme.text,
  },
  button: {
    padding: "0.4rem 0.8rem",
    borderRadius: radius,
    border: `1px solid ${theme.border}`,
    background: theme.buttonBg,
    color: theme.text,
    cursor: "pointer",
    fontSize: 13,
  },
  primary: {
    padding: "0.4rem 0.8rem",
    borderRadius: radius,
    border: `1px solid ${theme.accent}`,
    background: theme.accent,
    color: theme.onAccent,
    cursor: "pointer",
    fontSize: 13,
  },
};
