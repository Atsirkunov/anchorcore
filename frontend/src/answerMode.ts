// B58: per-answer degradation notes from the response flags. Unknown or
// missing flags (older backend) yield no banner — same principle as
// modelHealth.ts. Pure + unit-tested (answerMode.test.ts).
export function modeBannerNotes(mode?: string, retrieval?: string): string[] {
  const notes: string[] = [];
  if (mode === "context_only") notes.push("Context excerpts only — no answer model ran for this answer.");
  if (retrieval === "keyword_only") notes.push("Ranked by keywords only — embedding model unavailable.");
  return notes;
}

// R45.2: agent mode — the user's AI agent writes answers, by design. The Ask
// tab shows the ranked, cited evidence with a neutral hint, never the amber
// degraded banner (nothing is degraded).
export const AGENT_COMPOSE_HINT =
  "Agent mode — your AI agent writes the answers (via MCP or the anchorcore-search skill). These are the ranked, cited passages it would get. Ask your agent, or switch Settings → Who writes the answers to “A local model” for in-app answers.";

export function isAgentCompose(mode?: string): boolean {
  return mode === "agent_compose";
}
