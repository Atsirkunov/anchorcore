import { useState } from "react";
import { api } from "./api";
import type { PullProgress } from "./types";

export type PullSummary = {
  total: number;
  done: number;
  failed: string[];
  pct: number; // 0..100 across known byte totals
  busy: boolean;
};

// Pure roll-up of raw pull rows for progress bars; unit-tested.
export function summarizePulls(pulls: PullProgress[], wanted: string[]): PullSummary {
  const mine = pulls.filter((p) => wanted.includes(p.model));
  const done = mine.filter((p) => p.done && !p.error).length;
  const failed = mine.filter((p) => p.done && p.error).map((p) => p.model);
  let completed = 0;
  let total = 0;
  for (const p of mine) {
    completed += p.completed;
    total += p.total;
  }
  const pct = total > 0 ? Math.min(100, Math.round((completed / total) * 100)) : 0;
  return { total: wanted.length, done, failed, pct, busy: done + failed.length < wanted.length };
}

const POLL_MS = 800;
const MAX_POLLS = 1500; // ~20 min ceiling for multi-GB models

function delay(ms: number) {
  return new Promise((r) => setTimeout(r, ms));
}

// Starts a server-side pull and polls until every requested model reports done.
// Resolves to the finished rows (empty on transport failure — see pullError).
export function useModelPull() {
  const [pulls, setPulls] = useState<PullProgress[]>([]);
  const [pulling, setPulling] = useState(false);
  const [pullError, setPullError] = useState<string | null>(null);

  async function startPull(models: string[]): Promise<PullProgress[]> {
    setPulling(true);
    setPullError(null);
    setPulls([]);
    try {
      await api.pullModels(models);
      for (let i = 0; i < MAX_POLLS; i++) {
        await delay(POLL_MS);
        const res = await api.ollamaPulls();
        const mine = res.pulls.filter((p) => models.includes(p.model));
        setPulls(mine);
        if (mine.length === models.length && mine.every((p) => p.done)) return mine;
      }
      throw new Error("timed out waiting for the model download — check the connection and retry");
    } catch (e) {
      setPullError(e instanceof Error ? e.message : String(e));
      return [];
    } finally {
      setPulling(false);
    }
  }

  return { pulls, pulling, pullError, startPull };
}
