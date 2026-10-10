// B58: per-answer degradation notes from the response flags. Unknown or
// missing flags (older backend) yield no banner — same principle as
// modelHealth.ts. Pure + unit-tested (answerMode.test.ts).
export function modeBannerNotes(mode?: string, retrieval?: string): string[] {
  const notes: string[] = [];
  if (mode === "context_only") notes.push("Context excerpts only — no answer model ran for this answer.");
  if (retrieval === "keyword_only") notes.push("Ranked by keywords only — embedding model unavailable.");
  return notes;
}
