import { useEffect, useRef, useState } from "react";
import { api, RequestAbortedError } from "../api";
import { theme, radius } from "../theme";
import type { AskResponse, AskTurn } from "../types";
import { AnswerView, type AnswerTurn } from "./AnswerView";

// R23.4: the server warns in plain language whenever no answer model ran
// (`AskResponse.warning`, Some unless mode == "composed"). Carried per turn
// so degraded output can never render without the banner.
type Turn = AskTurn & { warning?: string | null };

const SAMPLE_QUESTIONS = [
  "What was decided about instant payments, and why?",
  "Who owns billing migration?",
  "Show the steps for it",
];

/** Question/answer pairs, newest first — the question reads above its answer. */
function pairs(turns: Turn[]): AnswerTurn[] {
  const out: AnswerTurn[] = [];
  for (let i = 0; i + 1 < turns.length; i += 2) {
    const a = turns[i + 1];
    out.push({ question: turns[i].content, answer: a.content, citations: a.citations ?? [], mode: a.mode, retrieval: a.retrieval, warning: a.warning });
  }
  return out.reverse();
}

export function AskTab({ projectId, onGoSources }: { projectId?: number; onGoSources?: () => void }) {
  const [turns, setTurns] = useState<Turn[]>([]);
  const [question, setQuestion] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [publicOnly, setPublicOnly] = useState(false);
  const [sourceCount, setSourceCount] = useState<number | null>(null);
  // folder roots, so citations read "specs/runbook.md" instead of an absolute path
  const [roots, setRoots] = useState<string[]>([]);
  const abortRef = useRef<AbortController | null>(null);

  useEffect(() => {
    let cancelled = false;
    api
      .listSources()
      .then(async (sources) => {
        if (cancelled) return;
        setSourceCount(sources.length);
        const folders = sources.filter((s) => s.connector === "folder");
        const configs = await Promise.all(folders.map((s) => api.sourceConfig(s.id).catch(() => ({}) as Record<string, string>)));
        if (!cancelled) setRoots(configs.map((c) => c.path).filter((p): p is string => !!p));
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
      const warning = (response as AskResponse).warning ?? null;
      setTurns((prev) => [
        ...prev,
        { role: "user", content: question },
        { role: "assistant", content: response.answer, citations: response.citations, mode: response.mode, retrieval: response.retrieval, warning },
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
        <h2 style={{ margin: 0 }}>Ask</h2>
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
          placeholder={turns.length ? "Follow-up… e.g. show the steps for it" : "What was decided about X, and why?"}
          style={{ flex: 1, padding: "0.8rem 1rem", borderRadius: radius, border: `1px solid ${theme.border}`, background: theme.bgCard, color: theme.text, fontSize: 14 }}
        />
        <button onClick={submit} disabled={busy} style={{ padding: "0.6rem 1.2rem", borderRadius: radius, border: "none", background: theme.accent, color: theme.onAccent, cursor: busy ? "not-allowed" : "pointer", opacity: busy ? 0.6 : 1 }}>
          {busy ? "Thinking…" : "Ask"}
        </button>
        {busy && (
          <button onClick={cancel} style={{ padding: "0.6rem 1rem", borderRadius: radius, border: `1px solid ${theme.redBorder}`, background: theme.redBg, color: theme.redText, cursor: "pointer" }}>
            Cancel
          </button>
        )}
      </div>
      <label
        style={{ display: "flex", alignItems: "center", gap: 6, fontSize: 12, color: theme.textDim, marginTop: 8, cursor: "pointer" }}
        title="Answer only from sources labelled 'public' — safe to share"
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
        <div style={{ display: "grid", gap: 28, marginTop: 28 }}>
          {pairs(turns).map((p, i) => (
            <AnswerView key={`${turns.length - i}-${p.question.slice(0, 24)}`} turn={p} roots={roots} latest={i === 0} />
          ))}
        </div>
      )}
      {turns.length === 0 && !busy && (
        <div style={styles.starter}>
          <p style={styles.starterLabel}>Try one</p>
          <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
            {SAMPLE_QUESTIONS.map((q) => (
              <button key={q} onClick={() => setQuestion(q)} style={styles.chip}>
                {q}
              </button>
            ))}
          </div>
          <p style={{ color: theme.textDim, margin: "12px 0 0", fontSize: 13 }}>
            Ask a question, then follow up naturally — e.g. "refund details" → "show the steps for it".
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
  clearButton: {
    background: "none",
    border: `1px solid ${theme.border}`,
    color: theme.textMuted,
    borderRadius: radius,
    padding: "0.25rem 0.6rem",
    cursor: "pointer",
    fontSize: 12,
  },
  starter: {
    marginTop: 28,
    paddingTop: 18,
    borderTop: `1px solid ${theme.border}`,
  },
  starterLabel: { margin: "0 0 8px", fontFamily: theme.mono, fontSize: 11, letterSpacing: "0.06em", textTransform: "uppercase", color: theme.textDim },
  chip: {
    background: theme.bgCard,
    border: `1px solid ${theme.border}`,
    color: theme.text,
    borderRadius: radius,
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
