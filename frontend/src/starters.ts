import type { ActionRecord, DecisionRecord } from "./types";

// Wave 1 (UX audit #4): starter questions fit the person's own data. The
// fixed sample questions only make sense on the bundled `sample/` corpus;
// anywhere else the starters come from their newest records, and none are
// shown when there is nothing to build them from.

export const SAMPLE_QUESTIONS = [
  "What was decided about instant payments, and why?",
  "Who owns billing migration?",
];

const MAX_TOPIC = 70;

/** True when every connected source is the bundled sample folder. */
export function onlySampleSources(roots: string[], sourceCount: number | null): boolean {
  if (!sourceCount || roots.length !== sourceCount) return false;
  return roots.every((r) => r.replace(/[\\/]+$/, "").split(/[\\/]/).pop() === "sample");
}

function topic(text: string): string | null {
  const t = text.replace(/^decision:\s*/i, "").replace(/\s+/g, " ").trim().replace(/[.?!:;]+$/, "");
  if (!t) return null;
  return t.length > MAX_TOPIC ? null : t;
}

function newestFirst<T extends { created_at: string }>(rows: T[]): T[] {
  return [...rows].sort((a, b) => b.created_at.localeCompare(a.created_at));
}

/** Up to two questions from the newest current decision and open action. */
export function startersFromRecords(decisions: DecisionRecord[], actions: ActionRecord[]): string[] {
  const out: string[] = [];
  const decision = newestFirst(decisions.filter((d) => d.current)).map((d) => topic(d.summary)).find(Boolean);
  if (decision) out.push(`What was decided about ${decision}, and why?`);
  const owner = newestFirst(actions.filter((a) => !a.done)).map((a) => a.owner.trim()).find(Boolean);
  if (owner) out.push(`What does ${owner} own?`);
  return out;
}
