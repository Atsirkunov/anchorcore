import type { PullProgress as Pull } from "../types";
import { summarizePulls } from "../useModelPull";
import { theme } from "../theme";
import { styles } from "./styles";

const MB = 1048576;

/** Progress for an in-app Ollama pull (`useModelPull`). */
export function PullProgress({ pulls, models, error }: { pulls: Pull[]; models: string[]; error: string | null }) {
  const summary = summarizePulls(pulls, models);
  return (
    <div style={{ marginTop: 6 }}>
      <div style={styles.progressTrack}>
        <div style={{ ...styles.progressBar, width: `${summary.pct}%` }} />
      </div>
      {pulls.map((p) => (
        <div key={p.model} style={{ fontSize: 12, marginTop: 4 }}>
          {p.model}:{" "}
          {p.done
            ? p.error
              ? `failed — ${p.error}`
              : "done"
            : p.total > 0
              ? `${p.status} — ${(p.completed / MB).toFixed(0)} / ${(p.total / MB).toFixed(0)} MB`
              : p.status}
        </div>
      ))}
      {error && <div style={{ fontSize: 12, color: theme.red, marginTop: 6 }}>{error}</div>}
    </div>
  );
}
