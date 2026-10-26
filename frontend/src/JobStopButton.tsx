import { commonStyles, theme } from "./theme";
import { jobStopLabel, jobStopTitle } from "./jobControlsText";
import type { Job } from "./types";
import type { StopNote } from "./useJobStop";

/** R30.4: one labeled Stop control per active job (`Stop reclassify #11`). */
export function JobStopButton({
  job,
  sourceName,
  busy,
  onStop,
}: {
  job: Job;
  sourceName?: string;
  busy: boolean;
  onStop: (job: Job) => void;
}) {
  const title = jobStopTitle(job, sourceName);
  return (
    <button
      onClick={() => onStop(job)}
      disabled={busy}
      aria-label={title}
      title={title}
      style={{ ...commonStyles.button, background: theme.redBg, color: theme.redText }}
    >
      {busy ? `Stopping #${job.id}…` : jobStopLabel(job)}
    </button>
  );
}

/** Outcome line under a job list (success in muted ink, failure in red). */
export function StopNoteLine({ note }: { note: StopNote }) {
  return (
    <div role="status" style={{ fontSize: 12, marginTop: 4, color: note.ok ? theme.textMuted : theme.red }}>
      {note.text}
    </div>
  );
}
