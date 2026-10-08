import { useEffect, useRef, useState } from "react";
import { api, RequestAbortedError } from "../api";
import { theme } from "../theme";
import type { AskTurn } from "../types";

const SAMPLE_QUESTIONS = [
  "What was decided about security transfers, and why?",
  "Who owns billing migration?",
  "Show the movements for it",
];

function Breadcrumb({ path }: { path?: string }) {
  if (!path) return null;
  const parts = path.split(" > ");
  return (
    <span style={{ fontSize: 11, color: theme.textDim }}>
      {parts.map((p, i) => (
        <span key={i}>
          {i > 0 && <span style={{ margin: "0 4px", color: theme.textMuted }}>›</span>}
          <span style={{ color: i === parts.length - 1 ? theme.text : theme.textDim }}>{p}</span>
        </span>
      ))}
    </span>
  );
}

function TagChips({ tags }: { tags?: string[] }) {
  if (!tags || tags.length === 0) return null;
  return (
    <span style={{ display: "inline-flex", gap: 4, flexWrap: "wrap", marginLeft: 6 }}>
      {tags.slice(0, 4).map((t) => (
        <span key={t} style={{ fontSize: 10, background: theme.bgHover, color: theme.accentAlt, border: `1px solid ${theme.border}`, padding: "0.1rem 0.4rem", borderRadius: 999 }}>{t}</span>
      ))}
    </span>
  );
}

export function AskTab({ projectId, onGoSources }: { projectId?: number; onGoSources?: () => void }) {
  const [turns, setTurns] = useState<AskTurn[]>([]);
  const [question, setQuestion] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [publicOnly, setPublicOnly] = useState(false);
  const [sourceCount, setSourceCount] = useState<number | null>(null);
  const abortRef = useRef<AbortController | null>(null);

  useEffect(() => {
    let cancelled = false;
    api
      .listSources()
      .then((sources) => {
        if (!cancelled) setSourceCount(sources.length);
      })
      .catch(() => {
        if (!cancelled) setSourceCount(null);
      });
    return () => {
      cancelled = true;
    };
  }, []);

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

      <div style={{ display: "flex", gap: 12 }}>
        <input
          value={question}
          onChange={(e) => setQuestion(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && submit()}
          placeholder={turns.length ? "Follow-up… e.g. show the movements for it" : "What was decided about X, and why?"}
          style={{ flex: 1, padding: "0.8rem 1rem", borderRadius: 10, border: `1px solid ${theme.border}`, background: theme.bgCard, color: theme.text, fontSize: 14 }}
        />
        <button onClick={submit} disabled={busy} style={{ padding: "0.6rem 1.2rem", borderRadius: 8, border: "none", background: theme.accent, color: theme.onAccent, cursor: busy ? "not-allowed" : "pointer", opacity: busy ? 0.6 : 1 }}>
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
        <div style={{ display: "flex", alignItems: "center", gap: 10, marginTop: 16, color: theme.textMuted, fontSize: 13 }}>
          <span style={{ display: "inline-flex", gap: 5 }}>
            {[0, 1, 2].map((d) => (
              <span
                key={d}
                className="animated"
                style={{
                  width: 8,
                  height: 8,
                  borderRadius: "50%",
                  background: theme.accentAlt,
                  display: "inline-block",
                  animation: "dotBounce 1.1s ease-in-out infinite",
                  animationDelay: `${d * 0.15}s`,
                }}
              />
            ))}
          </span>
          Gathering citations and composing an answer…
        </div>
      )}
      {error && <p style={{ color: theme.red }}>{error}</p>}

      {turns.length > 0 && (
        <div style={{ display: "grid", gap: 22, marginTop: 28 }}>
          {[...turns].reverse().map((t, i) => {
            const key = `${t.role}-${i}-${t.content.slice(0, 32)}`;
            const stagger = `${Math.min(i * 0.07, 0.35)}s`;
            return t.role === "user" ? (
              <div
                key={key}
                className="animated"
                style={{ ...styles.bubble, background: theme.bgHover, marginLeft: 64, animation: `rise 0.45s ease both`, animationDelay: stagger }}
              >
                <div style={{ fontSize: 11, color: theme.accentAlt, textTransform: "uppercase", letterSpacing: "0.06em", marginBottom: 6 }}>You</div>
                <div style={{ whiteSpace: "pre-wrap" }}>{t.content}</div>
              </div>
            ) : (
              <div
                key={key}
                className="animated"
                style={{ ...styles.bubble, background: theme.bgCard, animation: `popIn 0.45s ease both`, animationDelay: stagger }}
              >
                <div style={{ fontSize: 11, color: theme.purple, textTransform: "uppercase", letterSpacing: "0.06em", marginBottom: 6 }}>AnchorCore</div>
                <div style={{ whiteSpace: "pre-wrap", lineHeight: 1.7 }}>{t.content}</div>
                {t.citations && t.citations.length > 0 && (
                  <div style={{ marginTop: 14 }}>
                    <div style={{ fontSize: 12, color: theme.textMuted, textTransform: "uppercase", letterSpacing: "0.06em" }}>
                      Sources · {t.citations.length}
                    </div>
                    {t.citations.map((c, j) => (
                      <div
                        key={`${c.source_ref}-${j}`}
                        className="animated"
                        style={{ ...styles.cite, animation: `rise 0.4s ease both`, animationDelay: `${Math.min(j * 0.08, 0.4)}s` }}
                      >
                        <span style={{ color: theme.purple }}>[{c.kind}]</span> {c.summary}
                        <div style={{ color: theme.textDim, fontSize: 12 }}>{c.source_ref} · score {c.score}</div>
                        {c.path && (
                          <div style={{ marginTop: 3 }}>
                            <Breadcrumb path={c.path} />
                          </div>
                        )}
                        <TagChips tags={c.tags} />
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
        <div style={styles.starter}>
          <div style={{ fontWeight: 700, marginBottom: 8 }}>Try one:</div>
          <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
            {SAMPLE_QUESTIONS.map((q) => (
              <button key={q} onClick={() => setQuestion(q)} style={styles.chip}>
                {q}
              </button>
            ))}
          </div>
          <p style={{ color: theme.textDim, margin: "12px 0 0", fontSize: 13 }}>
            Ask a question, then follow up naturally — e.g. "merger details" → "show the movements for it".
          </p>
          {sourceCount === 0 && (
            <p style={{ margin: "8px 0 0", fontSize: 13, color: theme.textDim }}>
              No sources connected yet —{" "}
              <button onClick={onGoSources} style={styles.linkButton}>
                connect a folder in Sources
              </button>{" "}
              (try <code>sample/</code>), then ask.
            </p>
          )}
        </div>
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
    borderRadius: 14,
    padding: "1.25rem 1.4rem",
    fontSize: 14,
  },
  cite: {
    marginTop: 8,
    padding: "0.7rem 0.9rem",
    background: theme.bgElevated,
    borderRadius: 10,
    fontSize: 13,
    lineHeight: 1.6,
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
  starter: {
    marginTop: 24,
    background: theme.bgCard,
    border: `1px solid ${theme.border}`,
    borderRadius: 14,
    padding: "1.5rem 1.6rem",
  },
  chip: {
    background: theme.bgHover,
    border: `1px solid ${theme.border}`,
    color: theme.text,
    borderRadius: 999,
    padding: "0.35rem 0.8rem",
    cursor: "pointer",
    fontSize: 13,
  },
  linkButton: {
    background: "none",
    border: "none",
    padding: 0,
    color: theme.accentAlt,
    textDecoration: "underline",
    cursor: "pointer",
    fontSize: 13,
  },
};
