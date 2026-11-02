// B58: per-answer degradation notes from the response flags. Unknown or
// missing flags (older backend) yield no banner — same principle as
// modelHealth.ts. Pure + unit-tested (answerMode.test.ts).
//
// Wave 1 (UX audit #1): `context_only` no longer gets a note here — those
// turns render as the evidence view with CONTEXT_ONLY_HINT instead of a fake
// answer plus stacked warnings.
export function modeBannerNotes(_mode?: string, retrieval?: string): string[] {
  const notes: string[] = [];
  if (retrieval === "keyword_only") notes.push("Ranked by keywords only — embedding model unavailable.");
  return notes;
}

// R45.2: agent mode — the user's AI agent writes answers, by design. The Ask
// tab shows the ranked, cited evidence with a neutral hint, never the amber
// degraded banner (nothing is degraded).
export const AGENT_COMPOSE_HINT =
  "Agent mode — your AI agent writes the answers (via MCP or the ask-docs skill). These are the ranked, cited passages it would get. Ask your agent, or switch Settings → Who writes the answers to “A local model” for in-app answers.";

export function isAgentCompose(mode?: string): boolean {
  return mode === "agent_compose";
}

// Wave 1: local mode with no working answer model. The server returns
// matching context (`mode: "context_only"`); people see the cited passages,
// never that text dressed up as an answer.
export const CONTEXT_ONLY_HINT =
  "No answer model ran, so these are the passages that match your question. Set up an answer model in Settings to get written answers.";

export function isContextOnly(mode?: string): boolean {
  return mode === "context_only";
}

/** Turns shown as ranked passages instead of a written answer. */
export function isEvidenceView(mode?: string): boolean {
  return isAgentCompose(mode) || isContextOnly(mode);
}
