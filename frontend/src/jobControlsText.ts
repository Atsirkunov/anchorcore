import type { CancelJobResult, Job } from "./types";

/** R30.4: active = what a Stop control may target. */
export function isActiveJob(job: Job): boolean {
  return job.status === "running" || job.status === "pending";
}

/** R30.4: every Stop control names its target — kind + id (e.g. `Stop reclassify #11`). */
export function jobStopLabel(job: Pick<Job, "id" | "kind">): string {
  return `Stop ${job.kind} #${job.id}`;
}

/** Accessible name / tooltip: the label plus the source it belongs to. */
export function jobStopTitle(job: Pick<Job, "id" | "kind" | "status">, sourceName?: string): string {
  const where = sourceName ? ` on "${sourceName}"` : "";
  const what = job.status === "pending" ? "queued, cancels at once" : "running, stops after the current step";
  return `${jobStopLabel(job)}${where} (${what}; documents already processed are kept)`;
}

/** `cancelled — X/Y docs kept`: partial ingestion stays (docs/architecture.md, "Jobs and stopping them"). */
export function cancelledLine(processed: number, total: number): string {
  return `cancelled — ${processed}/${total} docs kept`;
}

/** Feedback after a successful Stop. */
export function stopSuccessText(job: Pick<Job, "id" | "kind">, res: CancelJobResult): string {
  if (res.was === "pending") return `#${job.id} ${job.kind} cancelled before it started`;
  return `#${job.id} ${job.kind} ${cancelledLine(res.processed, res.total)}`;
}

export function stopErrorText(job: Pick<Job, "id" | "kind">, err: unknown): string {
  const msg = err instanceof Error ? err.message : String(err);
  return `Stop ${job.kind} #${job.id} failed: ${msg}`;
}
