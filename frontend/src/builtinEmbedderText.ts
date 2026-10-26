import type { EmbedModelStatus } from "./types";

export const MB = 1024 * 1024;

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
