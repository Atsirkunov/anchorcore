import { useEffect, useRef, useState } from "react";
import { api, errMsg, RequestAbortedError } from "../api";
import { theme, radius } from "../theme";
import type { AskResponse, AskTurn, Project } from "../types";
import { AnswerView, type AnswerTurn } from "./AnswerView";
import { onlySampleSources, SAMPLE_QUESTIONS, startersFromRecords } from "../starters";

// R23.4: the server warns in plain language whenever no answer model ran
// (`AskResponse.warning`, Some unless mode == "composed"). Carried per turn
// so degraded output can never render without the banner.
type Turn = AskTurn & { warning?: string | null };

/** Question/answer pairs, newest first — the question reads above its answer. */
function pairs(turns: Turn[]): AnswerTurn[] {
  const out: AnswerTurn[] = [];
  for (let i = 0; i + 1 < turns.length; i += 2) {
    const a = turns[i + 1];
    out.push({ question: turns[i].content, answer: a.content, citations: a.citations ?? [], mode: a.mode, retrieval: a.retrieval, warning: a.warning });
  }
  return out.reverse();
}

export function AskTab({
  projectId,
  projects = [],
  onProject,
  onGoSources,
  onTour,
}: {
  projectId?: number;
  /** R49.9: the project scope moved here from the header. */
  projects?: Project[];
  onProject?: (id: number | null) => void;
  onGoSources?: () => void;
  onTour?: () => void;
}) {
  const [turns, setTurns] = useState<Turn[]>([]);
  const [question, setQuestion] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [publicOnly, setPublicOnly] = useState(false);
  const [sourceCount, setSourceCount] = useState<number | null>(null);
  // folder roots, so citations read "specs/runbook.md" instead of an absolute path
  const [roots, setRoots] = useState<string[]>([]);
  const abortRef = useRef<AbortController | null>(null);
  const starters = useStarters(roots, sourceCount, projectId);

  useEffect(() => {
    let cancelled = false;
    api
      .listSources()
      .then((sources) => {
        if (cancelled) return;
        setSourceCount(sources.length);
        // R48.14: folder roots come from the masked config on the list.
        const folders = sources.filter((s) => s.connector === "folder");
        setRoots(folders.map((s) => s.config?.path).filter((p): p is string => !!p));
      })
      .catch(() => {
        if (!cancelled) setSourceCount(null);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  async function submit(text: string = question) {
    if (!text.trim() || busy) return;
    setBusy(true);
    setError(null);
    const history = turns.slice(-6); // bound context; rewrite uses last 6 turns
    const controller = new AbortController();
    abortRef.current = controller;
    try {
      const response = await api.ask(text, history, projectId, controller.signal, publicOnly);
      const warning = (response as AskResponse).warning ?? null;
      setTurns((prev) => [
        ...prev,
        { role: "user", content: text },
        { role: "assistant", content: response.answer, citations: response.citations, mode: response.mode, retrieval: response.retrieval, warning },
      ]);
      setQuestion("");
    } catch (e) {
      // R48.7: only a real cancel says so; a timeout explains itself.
      setError(e instanceof RequestAbortedError ? "Question cancelled." : errMsg(e));
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
          <button onClick={clearContext} style={styles.clearButton} title="Forget the conversation. The next question is asked standalone">
            Clear context
          </button>
        )}
        {turns.length > 0 && (
          <span style={{ fontSize: 12, color: theme.textDim }}>
            {turns.length / 2} question{turns.length / 2 === 1 ? "" : "s"} in context. Follow-ups like "and what about its steps?" work
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
        <button onClick={() => submit()} disabled={busy} style={{ padding: "0.6rem 1.2rem", borderRadius: radius, border: "none", background: theme.accent, color: theme.onAccent, cursor: busy ? "not-allowed" : "pointer", opacity: busy ? 0.6 : 1 }}>
          {busy ? "Thinking…" : "Ask"}
        </button>
        {busy && (
          <button onClick={cancel} style={{ padding: "0.6rem 1rem", borderRadius: radius, border: `1px solid ${theme.redBorder}`, background: theme.redBg, color: theme.redText, cursor: "pointer" }}>
            Cancel
          </button>
        )}
      </div>
      <div style={styles.scopeRow}>
        {projects.length > 0 && (
          <label style={styles.scope}>
            Search in
            <select
              value={projectId ?? ""}
              onChange={(e) => onProject?.(e.target.value ? Number(e.target.value) : null)}
              style={styles.scopeSelect}
            >
              <option value="">All sources</option>
              {projects.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.name}
                  {p.is_default ? " (default)" : ""}
                </option>
              ))}
            </select>
          </label>
        )}
        <label style={{ ...styles.scope, cursor: "pointer" }} title="Answer only from sources labelled 'public', safe to share">
          <input type="checkbox" checked={publicOnly} onChange={(e) => setPublicOnly(e.target.checked)} />
          Public sources only
        </label>
      </div>
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
          {starters.length > 0 && (
            <>
              <p style={styles.starterLabel}>Try one</p>
              <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
                {starters.map((q) => (
                  <button
                    key={q}
                    onClick={() => {
                      setQuestion(q);
                      void submit(q);
                    }}
                    style={styles.chip}
                  >
                    {q}
                  </button>
                ))}
              </div>
            </>
          )}
          <p style={{ color: theme.textDim, margin: "12px 0 0", fontSize: 13 }}>
            Ask a question, then follow up naturally, e.g. "refund details" → "show the steps for it".
          </p>
          {sourceCount === 0 && (
            <p style={{ margin: "8px 0 0", fontSize: 13, color: theme.textDim }}>
              No sources connected yet.{" "}
              <button onClick={onGoSources} style={styles.linkButton}>
                Add a folder in the Library
              </button>{" "}
              (try <code>sample/</code>), then ask.
            </p>
          )}
          {onTour && (
            <p style={{ margin: "8px 0 0", fontSize: 13, color: theme.textDim }}>
              New here?{" "}
              <button onClick={onTour} style={styles.linkButton}>
                Take the tour
              </button>
            </p>
          )}
        </div>
      )}
    </div>
  );
}

/** Wave 1: the bundled sample's questions on the sample corpus only;
 *  otherwise up to two built from the person's newest records. */
function useStarters(roots: string[], sourceCount: number | null, projectId?: number): string[] {
  const sample = onlySampleSources(roots, sourceCount);
  const [fromRecords, setFromRecords] = useState<string[]>([]);
  useEffect(() => {
    if (sample || !sourceCount) return;
    let cancelled = false;
    Promise.all([api.listDecisions({ project_id: projectId }), api.listActions({ project_id: projectId, state: "open" })])
      .then(([d, a]) => {
        if (!cancelled) setFromRecords(startersFromRecords(d.decisions, a.actions));
      })
      .catch(() => {
        if (!cancelled) setFromRecords([]);
      });
    return () => {
      cancelled = true;
    };
  }, [sample, sourceCount, projectId]);
  return sample ? SAMPLE_QUESTIONS : fromRecords;
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
  scopeRow: { display: "flex", alignItems: "center", gap: 16, flexWrap: "wrap", marginTop: 8 },
  scope: { display: "flex", alignItems: "center", gap: 6, fontSize: 12, color: theme.textDim },
  scopeSelect: { background: theme.bgCard, border: `1px solid ${theme.border}`, color: theme.text, borderRadius: radius, padding: "0.2rem 0.4rem", fontSize: 12, maxWidth: 200 },
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
  starterLabel: { margin: "0 0 8px", fontFamily: theme.sans, fontWeight: 600, fontSize: 11, letterSpacing: "0.06em", textTransform: "uppercase", color: theme.textDim },
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
    color: theme.text,
    textDecoration: "underline",
    cursor: "pointer",
    fontSize: 13,
  },
};
