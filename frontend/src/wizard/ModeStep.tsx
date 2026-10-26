import { useEffect, useState } from "react";
import { api } from "../api";
import { BuiltinEmbedder } from "../BuiltinEmbedder";
import { theme } from "../theme";
import type { EmbedModelStatus, LlmMode, OnboardingState } from "../types";
import { useModelPull } from "../useModelPull";
import { PullProgress } from "./PullProgress";
import { styles } from "./styles";
import { isMissing } from "./wizardText";

type Props = {
  state: OnboardingState | null;
  mode: LlmMode;
  embedModel: string;
  onMode: (m: LlmMode) => void;
  recheck: () => Promise<void>;
};

const MODES: { id: LlmMode; title: string; body: string }[] = [
  {
    id: "agent",
    title: "My AI agent",
    body: "Claude Code, Codex, Cursor… write answers from cited passages, over MCP or the anchorcore-search skill. No answer model on this computer.",
  },
  {
    id: "local",
    title: "A local model on this computer",
    body: "AnchorCore writes answers itself with an Ollama model sized to this machine (or an OpenAI-compatible endpoint).",
  },
];

/** R45.6 step 1: who writes the answers, then search embeddings (built-in
 * model first; Ollama is optional and only makes indexing faster). */
export function ModeStep({ state, mode, embedModel, onMode, recheck }: Props) {
  const [embed, setEmbed] = useState<EmbedModelStatus | null>(null);
  const pull = useModelPull();

  // Re-read the active embedder whenever the onboarding checks change
  // (built-in download finished, Ollama started, a model pulled).
  useEffect(() => {
    api.embedModel().then(setEmbed).catch(() => {});
  }, [state]);

  const missing = state?.ollama.missing_models ?? [];
  const reachable = !!state?.ollama.reachable;
  const ollamaServes = embed?.active === "ollama" && reachable && !isMissing(missing, embedModel);
  const builtinServes = !!embed?.ready && embed.active === "builtin";
  const searchReady = ollamaServes || builtinServes;

  async function pullEmbed() {
    const done = await pull.startPull([embedModel]);
    if (done.length > 0) await recheck();
  }

  return (
    <div style={styles.body}>
      <div style={styles.label}>Who writes the answers?</div>
      <div style={styles.choices} role="radiogroup" aria-label="Who writes the answers?">
        {MODES.map((m) => (
          <button
            key={m.id}
            role="radio"
            aria-checked={mode === m.id}
            style={{ ...styles.choice, ...(mode === m.id ? styles.choiceActive : {}) }}
            onClick={() => onMode(m.id)}
          >
            <div style={styles.choiceTitle}>{m.title}</div>
            <div style={{ color: theme.textMuted }}>{m.body}</div>
          </button>
        ))}
      </div>
      <div style={styles.hint}>Switch any time in Settings — the index is the same in both modes.</div>

      <div style={styles.note}>
        <div style={{ ...styles.label, marginBottom: 6 }}>Search embeddings (needed in both modes)</div>
        <BuiltinEmbedder onReady={recheck} />
        {reachable && !ollamaServes && (
          <div style={{ marginTop: 10, fontSize: 13 }}>
            Ollama is running. {builtinServes ? "Optional: " : "Or "}pull <code>{embedModel}</code> into it for faster indexing — same
            vectors, nothing re-embeds when you switch.
            {!pull.pulling ? (
              <div>
                <button style={{ ...styles.button, ...styles.secondary, marginTop: 6 }} onClick={pullEmbed}>
                  Pull {embedModel} (~270 MB)
                </button>
              </div>
            ) : (
              <PullProgress pulls={pull.pulls} models={[embedModel]} error={null} />
            )}
            {pull.pullError && <div style={{ fontSize: 12, color: theme.red, marginTop: 6 }}>{pull.pullError}</div>}
          </div>
        )}
        {ollamaServes && (
          <div style={{ ...styles.checkRow, marginTop: 8, fontSize: 13 }}>
            <span style={{ ...styles.dot, background: theme.green }} />
            <span>Ollama serves {embedModel} — indexing uses it; the built-in model takes over when Ollama is off.</span>
          </div>
        )}
        {embed && !searchReady && (
          <div style={{ ...styles.hint, marginTop: 8 }}>
            You can continue now — search stays keyword-only until an embedding model is ready, then catches up on its own.
          </div>
        )}
      </div>
    </div>
  );
}
