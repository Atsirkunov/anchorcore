import { useQuery } from "@tanstack/react-query";
import { api } from "../api";
import type { Job } from "../types";

/** B37: one polling hook for ingestion jobs (replaces the per-tab interval
 * soup). Pauses polling when no jobs are running and when the tab is hidden.
 * `enabled=false` lets callers gate polling entirely (e.g. on tab visibility). */
export function useJobsPoll(options: { enabled?: boolean; refetchInterval?: number } = {}) {
  const { enabled = true, refetchInterval = 1000 } = options;
  return useQuery({
    queryKey: ["jobs"],
    queryFn: () => api.listJobs(undefined, 50),
    refetchInterval: (query) => {
      if (!enabled || document.hidden) return false;
      const jobs = query.state.data ?? [];
      return jobs.some((j: Job) => j.status === "running" || j.status === "pending") ? refetchInterval : false;
    },
  });
}
