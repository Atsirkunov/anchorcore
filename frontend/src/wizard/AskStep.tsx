import { useState } from "react";
import { AGENT_COMPOSE_HINT, isAgentCompose } from "../answerMode";
import { api } from "../api";
import { theme } from "../theme";
import type { Citation, LlmMode, OnboardingState } from "../types";
import { styles } from "./styles";

type Answer = { text: string; citations: Citation[]; mode?: string; warning?: string | null };

/** First question. Agent mode shows the cited passages an agent would get. */
export function AskStep({ state, mode }: { state: OnboardingState | null; mode: LlmMode }) {
  const [question, setQuestion] = useState("");
  const [answer, setAnswer] = useState<Answer | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function ask() {
    if (!question.trim() || busy) return;
    setBusy(true);
    setError(null);
    try {
      const res = await api.ask(question);
      const warning = (res as unknown as { warning?: string | null }).warning ?? null;
      setAnswer({ text: res.answer, citations: res.citations, mode: res.mode, warning });
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div style={styles.body}>
      <div style={styles.note}>
        {mode === "agent"
          ? "Ask something about what you just connected — you'll see the ranked, cited passages your agent receives."
          : "Ask a question about what you just connected — the answer cites its sources."}
      </div>
      {mode === "local" && state && !state.answer_ready && (
        <div style={styles.note}>Heads up: no working answer model yet — this answer will be excerpts. Go back to Answer model to set one up.</div>
      )}
      <div style={{ display: "flex", gap: 8 }}>
        <input
          style={styles.input}
          placeholder="e.g. What was decided about billing, and why?"
          value={question}
          onChange={(e) => setQuestion(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && ask()}
        />
        <button style={{ ...styles.button, ...styles.primary }} disabled={busy} onClick={ask}>
          {busy ? "Asking…" : "Ask"}
        </button>
      </div>
      {error && <div style={{ fontSize: 13, color: theme.red }}>{error}</div>}
      {answer && (
        <div style={styles.answerBox}>
          {answer.warning ? (
            <div style={styles.degradedNote}>{answer.warning}</div>
          ) : isAgentCompose(answer.mode) ? (
            <div style={{ ...styles.note, marginBottom: 10 }}>{AGENT_COMPOSE_HINT}</div>
          ) : answer.mode && answer.mode !== "composed" ? (
            <div style={styles.degradedNote}>Not a composed answer (mode: {answer.mode}) — go back to Answer model and set one up.</div>
          ) : null}
          {!isAgentCompose(answer.mode) && <div style={{ whiteSpace: "pre-wrap", lineHeight: 1.6 }}>{answer.text}</div>}
          {answer.citations.length > 0 && (
            <div style={{ marginTop: 10, fontSize: 12, color: theme.textMuted }}>
              Sources:
              {answer.citations.map((c, i) => (
                <div key={i} style={{ marginTop: 4 }}>
                  [{c.kind}] {c.summary} — {c.source_ref}
                </div>
              ))}
            </div>
          )}
        </div>
      )}
    </div>
  );
}
