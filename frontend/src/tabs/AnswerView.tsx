import { useState } from "react";
import { theme, radius } from "../theme";
import { AGENT_COMPOSE_HINT, isAgentCompose, modeBannerNotes } from "../answerMode";
import { excerpt, parseAnswer, relativeTo, splitSourceRef, type Inline } from "../answerText";
import type { Citation } from "../types";
import { ImageStrip } from "../ImageStrip";

/** One asked question and what came back: a written answer (local model) or
 *  the cited passages an agent composes from (R45.2 agent mode). */
export type AnswerTurn = {
  question: string;
  answer: string;
  citations: Citation[];
  mode?: string;
  retrieval?: string;
  warning?: string | null;
};

export function AnswerView({ turn, roots, latest }: { turn: AnswerTurn; roots: string[]; latest: boolean }) {
  const [active, setActive] = useState<number | null>(null);
  const agent = isAgentCompose(turn.mode);
  return (
    <article className="animated" style={{ ...styles.turn, ...(latest ? styles.latest : {}), animation: "popIn 0.45s ease both" }}>
      <p style={styles.question}>{turn.question}</p>
      {turn.warning ? <div style={styles.degradedNote}>{turn.warning}</div> : null}
      {modeBannerNotes(turn.mode, turn.retrieval).map((note) => (
        <div key={note} style={styles.degradedNote}>{note}</div>
      ))}
      {agent ? (
        <p style={styles.agentNote}>{AGENT_COMPOSE_HINT}</p>
      ) : (
        <div style={styles.answer}>
          <p style={styles.label}>Answer</p>
          <AnswerBody text={turn.answer} onCite={setActive} count={turn.citations.length} />
        </div>
      )}
      {turn.citations.length > 0 && (
        <div style={styles.sources}>
          <p style={styles.label}>{agent ? "Evidence for your agent" : "Sources"} · {turn.citations.length}</p>
          <ol style={styles.list}>
            {turn.citations.map((c, i) => (
              <SourceCard key={`${c.source_ref}-${i}`} c={c} n={i + 1} roots={roots} agent={agent} active={active === i + 1} />
            ))}
          </ol>
        </div>
      )}
    </article>
  );
}

function AnswerBody({ text, onCite, count }: { text: string; onCite: (n: number | null) => void; count: number }) {
  const inline = (parts: Inline[]) =>
    parts.map((p, i) => {
      if (p.t === "bold") return <strong key={i}>{p.v}</strong>;
      if (p.t === "text") return <span key={i}>{p.v}</span>;
      const known = p.n >= 1 && p.n <= count;
      return (
        <a
          key={i}
          href={known ? `#cite-${p.n}` : undefined}
          style={styles.mark}
          onMouseEnter={() => onCite(p.n)}
          onMouseLeave={() => onCite(null)}
          onFocus={() => onCite(p.n)}
          onBlur={() => onCite(null)}
        >
          [S{p.n}]
        </a>
      );
    });
  return (
    <>
      {parseAnswer(text).map((b, i) =>
        b.t === "p" ? (
          <p key={i} style={styles.para}>{inline(b.parts)}</p>
        ) : b.t === "ol" ? (
          <ol key={i} style={styles.answerList}>{b.items.map((it, j) => <li key={j}>{inline(it)}</li>)}</ol>
        ) : (
          <ul key={i} style={styles.answerList}>{b.items.map((it, j) => <li key={j}>{inline(it)}</li>)}</ul>
        ),
      )}
    </>
  );
}

function SourceCard({ c, n, roots, agent, active }: { c: Citation; n: number; roots: string[]; agent: boolean; active: boolean }) {
  const { location, section } = splitSourceRef(c.source_ref);
  const crumbs = (c.path ?? "").split(" > ").filter(Boolean);
  const title = crumbs.length ? crumbs.join(" › ") : c.item_title || relativeTo(location, roots);
  const lines = c.line_start && c.line_end ? `lines ${c.line_start}–${c.line_end}` : null;
  const where = [relativeTo(location, roots), section ? `§${section}` : null, lines].filter(Boolean).join(" · ");
  const text = excerpt(agent ? c.content || c.snippet : c.snippet || c.summary, agent ? 600 : 240);
  return (
    <li id={`cite-${n}`} style={{ ...styles.card, ...(active ? styles.cardActive : {}) }} title={`${c.kind} · score ${c.score}`}>
      <span style={styles.sid}>S{n}</span>
      <div style={{ minWidth: 0 }}>
        <div style={styles.cardTitle}>
          {title}
          {c.doc_type === "code" && <span style={styles.badge}>code</span>}
        </div>
        <div style={styles.where}>{where}</div>
        {text && <p style={styles.quote}>{text}</p>}
        {c.conflicts_with && c.conflicts_with.length > 0 && (
          <div style={{ color: theme.amber, fontSize: 12, marginTop: 4 }}>contradicts {c.conflicts_with.map((id) => `#${id}`).join(", ")}</div>
        )}
        {c.tags && c.tags.length > 0 && <div style={styles.tags}>{c.tags.slice(0, 4).join(" · ")}</div>}
        {c.images && c.images.length > 0 && <ImageStrip images={c.images} />}
      </div>
    </li>
  );
}

const styles: Record<string, React.CSSProperties> = {
  turn: { borderTop: `1px solid ${theme.border}`, paddingTop: 20 },
  latest: { borderTop: `1px solid ${theme.text}` },
  question: { margin: "0 0 14px", fontFamily: theme.serif, fontWeight: 500, fontSize: 20, lineHeight: 1.35 },
  label: { margin: "0 0 8px", fontFamily: theme.mono, fontSize: 11, letterSpacing: "0.06em", textTransform: "uppercase", color: theme.textDim },
  answer: { fontFamily: theme.serif, fontSize: 16, lineHeight: 1.65 },
  para: { margin: "0 0 10px" },
  answerList: { margin: "0 0 10px", paddingLeft: "1.4em" },
  mark: { fontFamily: theme.mono, fontSize: "0.72em", fontWeight: 600, color: theme.cite, verticalAlign: "0.35em", textDecoration: "none", padding: "0 1px" },
  sources: { marginTop: 16, paddingTop: 14, borderTop: `1px dashed ${theme.border}` },
  list: { listStyle: "none", margin: 0, padding: 0, display: "grid", gap: 8 },
  card: {
    display: "flex",
    gap: 12,
    padding: "0.65rem 0.85rem",
    border: `1px solid ${theme.border}`,
    borderRadius: radius,
    background: theme.bgCard,
    fontSize: 13,
    lineHeight: 1.5,
    transition: "border-color 0.2s ease, background 0.2s ease",
  },
  cardActive: { borderColor: theme.cite, background: theme.citeSoft },
  sid: { flexShrink: 0, fontFamily: theme.mono, fontWeight: 600, color: theme.cite, paddingTop: 1 },
  cardTitle: { fontWeight: 600 },
  badge: { marginLeft: 8, fontFamily: theme.mono, fontSize: 11, color: theme.blue },
  where: { fontFamily: theme.mono, fontSize: 12, color: theme.textDim, overflowWrap: "anywhere" },
  quote: { margin: "4px 0 0", fontFamily: theme.serif, fontSize: 14, color: theme.text, whiteSpace: "pre-wrap" },
  tags: { marginTop: 4, fontFamily: theme.mono, fontSize: 11, color: theme.textDim },
  degradedNote: {
    background: theme.amberBg,
    color: theme.amber,
    border: `1px solid ${theme.amber}`,
    borderRadius: radius,
    padding: "0.45rem 0.7rem",
    fontSize: 12,
    marginBottom: 10,
  },
  agentNote: { margin: "0 0 4px", fontSize: 13, lineHeight: 1.55, color: theme.textMuted },
};
