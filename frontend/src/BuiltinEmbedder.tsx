import { useCallback, useEffect, useRef, useState } from "react";
import { api } from "./api";
import { radius, theme } from "./theme";
import type { EmbedModelStatus } from "./types";

const MB = 1024 * 1024;

/** Download percentage, 0–100. */
export function downloadPct(p: EmbedModelStatus["progress"]): number {
  if (p.phase === "converting" || p.phase === "ready") return 100;
  if (p.total_bytes <= 0) return 0;
  return Math.min(100, Math.round((p.done_bytes / p.total_bytes) * 100));
}

/** One line describing where search embeddings come from right now. */
export function embedderLine(s: EmbedModelStatus): string {
  if (s.ready && s.active === "builtin") return `Built-in model ready (${s.model}${s.backend ? `, ${s.backend}` : ""}) — no Ollama needed for search.`;
  if (s.ready && s.setting === "ollama") return "Built-in model downloaded; search uses Ollama because Settings say so.";
  if (s.ready) return "Built-in model downloaded; Ollama is serving, so search uses it (faster indexing) — the built-in model takes over when Ollama is off.";
  if (s.setting === "builtin") return "Built-in model selected but not downloaded yet — search is keyword-only until it is.";
  return "Search embeddings come from Ollama. Download the built-in model to drop that requirement.";
}

/**
 * R45.4: built-in embedder card — status, the explicit download (never
 * automatic) and its progress. Used by Settings and the onboarding wizard.
 */
export function BuiltinEmbedder({ onReady }: { onReady?: () => void }) {
  const [status, setStatus] = useState<EmbedModelStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  // A ref, so an inline `onReady` from the parent can't re-trigger fetching.
  const onReadyRef = useRef(onReady);
  onReadyRef.current = onReady;

  const refresh = useCallback(() => {
    api
      .embedModel()
      .then((s) => {
        setStatus(s);
        if (s.ready) onReadyRef.current?.();
      })
      .catch((e) => setError(e instanceof Error ? e.message : String(e)));
  }, []);

  useEffect(() => refresh(), [refresh]);

  const phase = status?.progress.phase;
  const running = phase === "downloading" || phase === "converting";
  useEffect(() => {
    if (!running) return;
    const t = setInterval(refresh, 1000);
    return () => clearInterval(t);
  }, [running, refresh]);

  async function download() {
    setError(null);
    try {
      await api.downloadEmbedModel();
      refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  if (!status) return error ? <div style={{ fontSize: 12, color: theme.red }}>{error}</div> : null;
  const p = status.progress;
  return (
    <div style={{ fontSize: 13 }}>
      <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
        <span style={{ width: 8, height: 8, borderRadius: "50%", background: status.ready ? theme.green : theme.amber, flex: "none" }} />
        <span>{embedderLine(status)}</span>
      </div>
      {!status.ready && !running && (
        <button
          style={{ marginTop: 6, padding: "6px 12px", borderRadius: radius, border: "none", background: theme.accent, color: theme.onAccent, cursor: "pointer" }}
          onClick={download}
        >
          Download built-in model ({Math.round(status.download_bytes / MB)} MB, {Math.round(status.download_bytes / MB / 2)} MB on disk)
        </button>
      )}
      {running && (
        <div style={{ marginTop: 6 }}>
          <div style={{ background: theme.bg, border: `1px solid ${theme.border}`, borderRadius: radius, height: 8, overflow: "hidden" }}>
            <div style={{ background: theme.accentAlt, height: "100%", width: `${downloadPct(p)}%` }} />
          </div>
          <div style={{ fontSize: 12, color: theme.textDim, marginTop: 4 }}>
            {phase === "converting"
              ? "Converting to f16…"
              : `${p.file ?? ""} — ${Math.round(p.done_bytes / MB)} / ${Math.round(p.total_bytes / MB)} MB`}
          </div>
        </div>
      )}
      {phase === "failed" && p.error && <div style={{ fontSize: 12, color: theme.red, marginTop: 6 }}>Download failed: {p.error}</div>}
      {error && <div style={{ fontSize: 12, color: theme.red, marginTop: 6 }}>{error}</div>}
      {!status.ready && status.dir && (
        <div style={{ fontSize: 12, color: theme.textDim, marginTop: 6 }}>
          Offline? Put <code>config.json</code>, <code>tokenizer.json</code> and <code>model.safetensors</code> from {status.model} (revision{" "}
          <code>{status.revision.slice(0, 7)}</code>) in <code>{status.dir}</code>, then press Download — present files are checked first.
        </div>
      )}
    </div>
  );
}
