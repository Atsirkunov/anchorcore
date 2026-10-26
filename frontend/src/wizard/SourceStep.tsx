import { useState } from "react";
import { api } from "../api";
import { FolderPicker } from "../FolderPicker";
import { theme } from "../theme";
import type { OnboardingState } from "../types";
import { styles } from "./styles";

function delay(ms: number) {
  return new Promise((r) => setTimeout(r, ms));
}

/** Connect a folder (or the bundled sample) and wait for the first sync. */
export function SourceStep({ state, onSynced }: { state: OnboardingState | null; onSynced: () => void }) {
  const [folderPath, setFolderPath] = useState("");
  const [pickerOpen, setPickerOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [progress, setProgress] = useState<{ total: number; processed: number } | null>(null);
  const [error, setError] = useState<string | null>(null);

  async function runSync(path: string, name: string) {
    setBusy(true);
    setError(null);
    setProgress(null);
    try {
      const source = await api.createSource({ connector: "folder", name, config: { path } });
      const job = await api.syncSource(source.id);
      for (;;) {
        await delay(900);
        const j = await api.getJob(job.id);
        setProgress({ total: j.total, processed: j.processed });
        if (j.status === "failed") throw new Error(j.error || "sync failed");
        if (j.status !== "running" && j.status !== "pending") break;
      }
      onSynced();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  const connectFolder = () => folderPath.trim() && runSync(folderPath.trim(), "My folder");

  return (
    <div style={styles.body}>
      <div style={styles.note}>
        AnchorCore ingests a folder of notes, code and business docs (Markdown, source code, Office, OpenDocument, RTF, email, PDF) — cloned git
        repos work like any folder, honoring .gitignore.
        {state?.sample.available ? " You can also use the bundled sample corpus to try it." : null}
      </div>
      {state && state.sources_count > 0 && <div style={styles.hint}>You already have {state.sources_count} source(s) — Continue, or add another.</div>}
      {state?.sample.available && state.sample.path && (
        <button style={{ ...styles.button, background: theme.citeSoft, color: theme.accentAlt }} disabled={busy} onClick={() => runSync(state.sample.path!, "Sample company memory")}>
          {busy ? "Syncing…" : "Try the sample corpus"}
        </button>
      )}
      <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
        <input
          style={{ ...styles.input, minWidth: 220 }}
          placeholder="Or enter a folder path, e.g. /Users/me/Documents/notes"
          value={folderPath}
          onChange={(e) => setFolderPath(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && connectFolder()}
        />
        <button style={styles.button} disabled={busy} title="Browse folders on this machine" onClick={() => setPickerOpen((v) => !v)}>
          Browse…
        </button>
        <button style={{ ...styles.button, ...styles.primary }} disabled={busy} onClick={connectFolder}>
          Connect
        </button>
      </div>
      {pickerOpen && (
        <FolderPicker
          initialPath={folderPath}
          onSelect={(p) => {
            setFolderPath(p);
            setPickerOpen(false);
          }}
          onCancel={() => setPickerOpen(false)}
        />
      )}
      {busy && (
        <div style={{ fontSize: 13, color: theme.accentAlt }}>
          {progress && progress.total > 0 ? `Ingesting… ${progress.processed}/${progress.total}` : "Ingesting…"}
        </div>
      )}
      {error && <div style={{ fontSize: 13, color: theme.red }}>{error}</div>}
    </div>
  );
}
