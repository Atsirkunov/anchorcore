import { useCallback, useEffect, useRef, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { api } from "./api";
import { stopErrorText, stopSuccessText } from "./jobControlsText";
import type { Job } from "./types";

export type StopNote = { ok: boolean; text: string };

/** A document already past its last checkpoint still finishes after Stop,
 * so the kept count is re-read once the task has had time to stop. */
const SETTLE_MS = 1500;

/** R30.4: per-job Stop state — which cancels are in flight (their controls
 * disable) and the last outcome per job (success line or error), kept after
 * the job leaves the active list so the feedback stays visible. */
export function useJobStop(onStopped?: () => void) {
  const queryClient = useQueryClient();
  const [inFlight, setInFlight] = useState<ReadonlySet<number>>(new Set());
  const [notes, setNotes] = useState<Record<number, StopNote>>({});
  const timers = useRef<number[]>([]);
  useEffect(() => () => timers.current.forEach((t) => window.clearTimeout(t)), []);

  const settle = useCallback((job: Job) => {
    const t = window.setTimeout(() => {
      api
        .getJob(job.id)
        .then((fresh) => {
          const text = stopSuccessText(job, { cancelled: true, job_id: job.id, kind: job.kind, was: "running", status: "cancelled", processed: fresh.processed, total: fresh.total });
          setNotes((n) => ({ ...n, [job.id]: { ok: true, text } }));
        })
        .catch(() => {});
    }, SETTLE_MS);
    timers.current.push(t);
  }, []);

  const stop = useCallback(
    async (job: Job) => {
      setInFlight((s) => new Set(s).add(job.id));
      try {
        const res = await api.cancelJob(job.id);
        setNotes((n) => ({ ...n, [job.id]: { ok: true, text: stopSuccessText(job, res) } }));
        if (res.was === "running") settle(job);
        onStopped?.();
      } catch (e) {
        setNotes((n) => ({ ...n, [job.id]: { ok: false, text: stopErrorText(job, e) } }));
      } finally {
        setInFlight((s) => {
          const next = new Set(s);
          next.delete(job.id);
          return next;
        });
        await Promise.all([
          queryClient.invalidateQueries({ queryKey: ["jobs"] }),
          queryClient.invalidateQueries({ queryKey: ["running-jobs"] }),
        ]);
      }
    },
    [onStopped, queryClient, settle],
  );

  return { stop, inFlight, notes };
}
