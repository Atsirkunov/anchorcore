import { useState } from "react";
import { api } from "../api";
import type { AskTurn } from "../types";

export function AskTab({ projectId }: { projectId?: number }) {
  const [turns, setTurns] = useState<AskTurn[]>([]);
  const [question, setQuestion] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function submit() {
    if (!question.trim() || busy) return;
    setBusy(true);
    setError(null);
    const history = turns.slice(-6); // bound context; rewrite uses last 6 turns
    try {
      const response = await api.ask(question, history, projectId);
      setTurns((prev) => [
        ...prev,
        { role: "user", content: question },
        { role: "assistant", content: response.answer, citations: response.citations },
      ]);
      setQuestion("");
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  function clearContext() {
    setTurns([]);
    setError(null);
  }

  return (
    <div>
      <div style={{ display: "flex", alignItems: "center", gap: 12, marginBottom: 12 }}>
        <h2 style={{ margin: 0 }}>Ask your memory</h2>
        {turns.length > 0 && (
          <button onClick={clearContext} style={styles.clearButton} title="Forget the conversation — the next question is asked standalone">
            Clear context
          </button>
        )}
        {turns.length > 0 && (
          <span style={{ fontSize: 12, color: "#6b7280" }}>
            {turns.length / 2} question{turns.length / 2 === 1 ? "" : "s"} in context — follow-ups like "and what about its movements?" work
          </span>
        )}
      </div>

      <div style={{ display: "flex", gap: 8 }}>
        <input
          value={question}
          onChange={(e) => setQuestion(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && submit()}
          placeholder={turns.length ? "Follow-up… e.g. show the movements for it" : "What was decided about X, and why?"}
          style={{ flex: 1, padding: "0.6rem 0.8rem", borderRadius: 8, border: "1px solid #2d333b", background: "#171a21", color: "#e6e8eb" }}
        />
        <button onClick={submit} disabled={busy} style={{ padding: "0.6rem 1.2rem", borderRadius: 8, border: "none", background: "#6366f1", color: "#fff", cursor: busy ? "not-allowed" : "pointer", opacity: busy ? 0.6 : 1 }}>
          {busy ? "Thinking…" : "Ask"}
        </button>
      </div>
      {busy && (
        <div style={{ display: "flex", alignItems: "center", gap: 8, marginTop: 12, color: "#9ca3af", fontSize: 13 }}>
          <span style={styles.spinner} />
          Retrieving context and composing an answer…
        </div>
      )}
      {error && <p style={{ color: "#f87171" }}>{error}</p>}

      {turns.length > 0 && (
        <div style={{ display: "grid", gap: 12, marginTop: 16 }}>
          {[...turns].reverse().map((t, i) =>
            t.role === "user" ? (
              <div key={i} style={{ ...styles.bubble, background: "#1e2430", marginLeft: 48 }}>
                <div style={{ fontSize: 11, color: "#7dd3fc", textTransform: "uppercase", marginBottom: 4 }}>You</div>
                <div style={{ whiteSpace: "pre-wrap" }}>{t.content}</div>
              </div>
            ) : (
              <div key={i} style={{ ...styles.bubble, background: "#171a21" }}>
                <div style={{ fontSize: 11, color: "#8b5cf6", textTransform: "uppercase", marginBottom: 4 }}>AnchorCore</div>
                <div style={{ whiteSpace: "pre-wrap", lineHeight: 1.6 }}>{t.content}</div>
                {t.citations && t.citations.length > 0 && (
                  <div style={{ marginTop: 10 }}>
                    <div style={{ fontSize: 12, color: "#9ca3af", textTransform: "uppercase" }}>Sources</div>
                    {t.citations.map((c, j) => (
                      <div key={j} style={{ marginTop: 6, padding: "0.5rem 0.75rem", background: "#14171d", borderRadius: 6, fontSize: 13 }}>
                        <span style={{ color: "#8b5cf6" }}>[{c.kind}]</span> {c.summary}
                        <div style={{ color: "#6b7280", fontSize: 12 }}>{c.source_ref} · score {c.score}</div>
                      </div>
                    ))}
                  </div>
                )}
              </div>
            )
          )}
        </div>
      )}
      {turns.length === 0 && !busy && (
        <p style={{ color: "#6b7280", marginTop: 16 }}>
          Ask a question, then follow up naturally — e.g. "merger details" → "show the movements for it".
        </p>
      )}
    </div>
  );
}

const styles: Record<string, React.CSSProperties> = {
  spinner: {
    width: 14,
    height: 14,
    borderRadius: "50%",
    border: "2px solid #2d333b",
    borderTopColor: "#7dd3fc",
    animation: "spin 0.8s linear infinite",
    flexShrink: 0,
  },
  bubble: {
    border: "1px solid #2d333b",
    borderRadius: 10,
    padding: "0.85rem 1rem",
    fontSize: 14,
  },
  clearButton: {
    background: "none",
    border: "1px solid #6b3030",
    color: "#fca5a5",
    borderRadius: 6,
    padding: "0.25rem 0.6rem",
    cursor: "pointer",
    fontSize: 12,
  },
};
