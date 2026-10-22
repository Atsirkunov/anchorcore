import type { Health } from "./types";

// B54 — dummy-proof model banner: the degraded-model issues in one pure,
// unit-tested place. Missing fields fall back to the pre-B54 signals so the
// banner still works against an older backend.
//
// R45.17: the Ollama-offline line used to fire unconditionally, claiming
// broken classification and basic answers even when an agent or cloud model
// composes answers (and classification is rules-based by default since
// R45.1, so Ollama never affected it). Now each line only claims what is
// actually down: answers hinge on the mode-aware `answer_ready`, search on
// the embed provider.
export function modelIssues(health: Health | undefined): string[] {
  if (!health) return [];
  const c = health.components;
  const issues: string[] = [];
  const ready = c.answer_ready ?? c.answer_key !== "missing";
  if (c.ollama === "offline") {
    const answersDown = !ready;
    // Absent (old backend) counts as local: the narrow note is the safe side.
    const searchDown = c.embedder?.provider !== "cloud";
    if (answersDown && searchDown) {
      issues.push("Ollama is offline — local answers stay basic and search stays keyword-only.");
    } else if (answersDown) {
      issues.push("Ollama is offline — local answers stay basic.");
    } else if (searchDown) {
      issues.push("Ollama is offline — answers are unaffected, but search stays keyword-only until it's back.");
    }
    // Cloud answers + cloud embeddings: Ollama is irrelevant — silent.
  }
  const missing = c.missing_models ?? [];
  if (missing.length > 0) {
    issues.push(
      ready
        ? `Models not installed yet (${missing.join(", ")}) — install them for full search quality.`
        : `Models not installed yet (${missing.join(", ")}) — install them for full answers.`,
    );
  }
  if (!ready) {
    issues.push("No working answer model — answers show context excerpts only, not composed answers.");
  }
  return issues;
}
