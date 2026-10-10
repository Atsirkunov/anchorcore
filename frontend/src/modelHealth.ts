import type { Health } from "./types";

// B54 — dummy-proof model banner: the degraded-model issues in one pure,
// unit-tested place. Missing fields fall back to the pre-B54 signals so the
// banner still works against an older backend.
export function modelIssues(health: Health | undefined): string[] {
  if (!health) return [];
  const c = health.components;
  const issues: string[] = [];
  if (c.ollama === "offline") {
    issues.push("Ollama is offline — classification falls back to simple rules and local answers stay basic.");
  }
  const missing = c.missing_models ?? [];
  if (missing.length > 0) {
    issues.push(`Models not installed yet (${missing.join(", ")}) — install them for full answers.`);
  }
  const ready = c.answer_ready ?? c.answer_key !== "missing";
  if (!ready) {
    issues.push("No working answer model — answers show context excerpts only, not composed answers.");
  }
  return issues;
}
