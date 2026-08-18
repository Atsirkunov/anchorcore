import { useRef, useState } from "react";
import { api, RequestAbortedError } from "../api";
import { theme } from "../theme";
import type { AskTurn } from "../types";

export function AskTab({ projectId }: { projectId?: number }) {
  const [turns, setTurns] = useState<AskTurn[]>([]);
  const [question, setQuestion] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [publicOnly, setPublicOnly] = useState(false);
  const abortRef = useRef<AbortController | null>(null);

  async function submit() {
    if (!question.trim() || busy) return;
    setBusy(true);
    setError(null);
    const history = turns.slice(-6); // bound context; rewrite uses last 6 turns
    const controller = new AbortController();
    abortRef.current = controller;
    try {
      const response = await api.ask(question, history, projectId, controller.signal, publicOnly);
      setTurns((prev) => [
        ...prev,
        { role: "user", content: question },
        { role: "assistant", content: response.answer, citations: response.citations },
      ]);
      setQuestion("");
    } catch (e) {
      if (e instanceof RequestAbortedError) {
        setError("Question cancelled.");
      } else {
        setError(e instanceof Error ? e.message : String(e));
      }
    } finally {
      abortRef.current = null;
      setBusy(false);
    }
  }

  function cancel() {
    abortRef.current?.abort();
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
          <span style={{ fontSize: 12, color: theme.textDim }}>
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
          style={{ flex: 1, padding: "0.6rem 0.8rem", borderRadius: 8, border: `1px solid ${theme.border}`, background: theme.bgCard, color: theme.text }}
        />
        <button onClick={submit} disabled={busy} style={{ padding: "0.6rem 1.2rem", borderRadius: 8, border: "none", background: theme.accent, color: "#fff", cursor: busy ? "not-allowed" : "pointer", opacity: busy ? 0.6 : 1 }}>
          {busy ? "Thinking…" : "Ask"}
        </button>
        {busy && (
          <button onClick={cancel} style={{ padding: "0.6rem 1rem", borderRadius: 8, border: `1px solid ${theme.redBorder}`, background: theme.redBg, color: theme.redText, cursor: "pointer" }}>
            Cancel
          </button>
        )}
      </div>
      <label
        style={{ display: "flex", alignItems: "center", gap: 6, fontSize: 12, color: theme.textDim, marginTop: 8, cursor: "pointer" }}
        title="Answer only from sources labelled 'public' — safe to share (B30)"
      >
        <input type="checkbox" checked={publicOnly} onChange={(e) => setPublicOnly(e.target.checked)} />
        Public-only — answer from public sources only (share/MCP-safe)
      </label>
      {busy && (
        <div style={{ display: "flex", alignItems: "center", gap: 8, marginTop: 12, color: theme.textMuted, fontSize: 13 }}>
          <span style={styles.spinner} />
          Retrieving context and composing an answer…
        </div>
      )}
      {error && <p style={{ color: theme.red }}>{error}</p>}

      {turns.length > 0 && (
        <div style={{ display: "grid", gap: 12, marginTop: 16 }}>
          {[...turns].reverse().map((t, i) => {
            const key = `${t.role}-${i}-${t.content.slice(0, 32)}`;
            return t.role === "user" ? (
              <div key={key} style={{ ...styles.bubble, background: theme.bgHover, marginLeft: 48 }}>
                <div style={{ fontSize: 11, color: theme.accentAlt, textTransform: "uppercase", marginBottom: 4 }}>You</div>
                <div style={{ whiteSpace: "pre-wrap" }}>{t.content}</div>
              </div>
            ) : (
              <div key={key} style={{ ...styles.bubble, background: theme.bgCard }}>
                <div style={{ fontSize: 11, color: theme.purple, textTransform: "uppercase", marginBottom: 4 }}>AnchorCore</div>
                <div style={{ whiteSpace: "pre-wrap", lineHeight: 1.6 }}>{t.content}</div>
                {t.citations && t.citations.length > 0 && (
                  <div style={{ marginTop: 10 }}>
                    <div style={{ fontSize: 12, color: theme.textMuted, textTransform: "uppercase" }}>Sources</div>
                    {t.citations.map((c, j) => (
                      <div key={`${c.source_ref}-${j}`} style={{ marginTop: 6, padding: "0.5rem 0.75rem", background: theme.bgElevated, borderRadius: 6, fontSize: 13 }}>
                        <span style={{ color: theme.purple }}>[{c.kind}]</span> {c.summary}
                        <div style={{ color: theme.textDim, fontSize: 12 }}>{c.source_ref} · score {c.score}</div>
                      </div>
                    ))}
                  </div>
                )}
              </div>
            );
          })}
        </div>
      )}
      {turns.length === 0 && !busy && (
        <p style={{ color: theme.textDim, marginTop: 16 }}>
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
    border: `2px solid ${theme.border}`,
    borderTopColor: theme.accentAlt,
    animation: "spin 0.8s linear infinite",
    flexShrink: 0,
  },
  bubble: {
    border: `1px solid ${theme.border}`,
    borderRadius: 10,
    padding: "0.85rem 1rem",
    fontSize: 14,
  },
  clearButton: {
    background: "none",
    border: `1px solid ${theme.redBorder}`,
    color: theme.redText,
    borderRadius: 6,
    padding: "0.25rem 0.6rem",
    cursor: "pointer",
    fontSize: 12,
  },
};
