import { useCallback, useEffect, useState } from "react";
import { api } from "../api";
import { theme } from "../theme";
import type { Entity, MergeProposal } from "../types";

type EntityContext = { source_name: string | null; source_ref: string; window_text: string; expanded_before: string[]; expanded_after: string[]; full_text: string; highlight: string; item_title: string };

function highlight(text: string, needle: string) {
  if (!needle || !text.includes(needle)) return null;
  const i = text.indexOf(needle);
  return (
    <>
      {text.slice(0, i)}
      <mark style={{ background: theme.amber, color: theme.markText, padding: "0 2px", borderRadius: 3 }}>{needle}</mark>
      {text.slice(i + needle.length)}
    </>
  );
}

export function ReviewTab() {
  const [low, setLow] = useState<Entity[]>([]);
  const [dupes, setDupes] = useState<MergeProposal[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [busyEntity, setBusyEntity] = useState<number | null>(null);
  const [busyProposal, setBusyProposal] = useState<number | null>(null);
  const [expanded, setExpanded] = useState<number | null>(null);
  const [ctx, setCtx] = useState<Record<number, EntityContext>>({});
  const [showFull, setShowFull] = useState<Record<number, boolean>>({});
  const [disputeFor, setDisputeFor] = useState<number | null>(null);
  const [disputeReason, setDisputeReason] = useState("");

  const refresh = useCallback(() => {
    api.lowConfidence().then(setLow).catch((e) => setError(String(e)));
    api.duplicates().then(setDupes).catch((e) => setError(String(e)));
  }, []);

  useEffect(() => refresh(), [refresh]);

  async function toggleContext(e: Entity) {
    const next = expanded === e.id ? null : e.id;
    setExpanded(next);
    if (next !== null && !ctx[e.id]) {
      try {
        const c = await api.entityContext(e.id);
        setCtx((m) => ({ ...m, [e.id]: c }));
      } catch {
        // fallback to window_text already on entity
      }
    }
  }

  async function decide(proposalId: number, decision: "merge" | "dismiss") {
    setBusyProposal(proposalId);
    setError(null);
    try {
      await api.decideMerge(proposalId, decision);
      refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusyProposal(null);
    }
  }

  async function updateEntity(entityId: number, patch: Partial<Pick<Entity, "status">>) {
    setBusyEntity(entityId);
    setError(null);
    try {
      await api.updateEntity(entityId, patch);
      refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusyEntity(null);
    }
  }

  async function recordDispute(entityId: number) {
    setBusyEntity(entityId);
    setError(null);
    try {
      await api.disputeEntity(entityId, disputeReason);
      setDisputeFor(null);
      setDisputeReason("");
      refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusyEntity(null);
    }
  }

  return (
    <div>
      <h2>Review</h2>
      {error && <p style={{ color: theme.red }}>{error}</p>}

      <h3 style={{ color: theme.textMuted }}>Low confidence — needs a human look</h3>
      <div style={{ display: "grid", gap: 8, marginBottom: 24 }}>
        {low.map((e) => (
          <article key={e.id} style={{ background: theme.bgCard, border: `1px solid ${theme.border}`, borderRadius: 8, padding: "0.75rem 1rem" }}>
            <div style={{ fontSize: 12, color: theme.textMuted }}>
              [{e.kind}] conf {(e.confidence * 100).toFixed(0)}% · {e.source_ref}
              {ctx[e.id]?.item_title && <span> · {ctx[e.id].item_title}</span>}
              {ctx[e.id]?.source_name && <span style={{ color: theme.accentAlt }}> · {ctx[e.id].source_name}</span>}
              {e.dispute_count > 0 && (
                <span title={`Disputed ${e.dispute_count}× — excluded from Q&A`} style={{ marginLeft: 6, color: theme.redText }}>
                  ⚑ disputed ×{e.dispute_count}
                </span>
              )}
            </div>
            <p style={{ margin: "0.3rem 0", fontWeight: 600 }}>{e.summary}</p>
            {e.reasoning && <p style={{ margin: 0, color: theme.textMuted, fontSize: 13 }}>{e.reasoning}</p>}
            {(e.window_text || ctx[e.id]) && (
              <div style={{ marginTop: 8 }}>
                <button onClick={() => toggleContext(e)} style={styles.contextButton}>
                  {expanded === e.id ? "Hide context ▲" : "Show context — file, section & neighbours ▼"}
                </button>
                {expanded === e.id && (
                  <div style={styles.contextBox}>
                    <div style={{ fontSize: 11, color: theme.textDim, marginBottom: 6, textTransform: "uppercase" }}>
                      {ctx[e.id]?.item_title || "Source"} · {ctx[e.id]?.source_ref || e.source_ref}
                      {ctx[e.id]?.source_name ? ` · ${ctx[e.id].source_name}` : ""}
                    </div>
                    {ctx[e.id]?.expanded_before.map((t, i) => (
                      <pre key={`b-${i}`} style={styles.contextPreDim}>
                        {t}
                      </pre>
                    ))}
                    <pre style={styles.contextPre}>
                      {highlight(ctx[e.id]?.window_text || e.window_text || "", ctx[e.id]?.highlight || e.summary) ||
                        ctx[e.id]?.window_text ||
                        e.window_text}
                    </pre>
                    {ctx[e.id]?.expanded_after.map((t, i) => (
                      <pre key={`a-${i}`} style={styles.contextPreDim}>
                        {t}
                      </pre>
                    ))}
                    {ctx[e.id]?.full_text && (
                      <div style={{ marginTop: 8 }}>
                        <button onClick={() => setShowFull((m) => ({ ...m, [e.id]: !m[e.id] }))} style={styles.contextButton}>
                          {showFull[e.id] ? "Hide full document ▲" : "Show full document (collapsed) ▼"}
                        </button>
                        {showFull[e.id] && (
                          <pre style={{ ...styles.contextBox, marginTop: 6, maxHeight: 400, background: theme.bg }}>{ctx[e.id].full_text}</pre>
                        )}
                      </div>
                    )}
                    {!ctx[e.id] && <div style={{ fontSize: 12, color: theme.textDim }}>Loading context…</div>}
                  </div>
                )}
              </div>
            )}
            <div style={{ display: "flex", gap: 8, marginTop: 8 }}>
              <button
                style={{ ...styles.button, ...(busyEntity !== null ? styles.disabled : {}) }}
                disabled={busyEntity !== null}
                onClick={() => updateEntity(e.id, { status: "verified" })}
              >
                {busyEntity === e.id ? "…" : "Looks right"}
              </button>
              {disputeFor !== e.id ? (
                <button
                  style={{ ...styles.button, ...(busyEntity !== null ? styles.disabled : {}) }}
                  disabled={busyEntity !== null}
                  onClick={() => {
                    setDisputeFor(e.id);
                    setDisputeReason("");
                  }}
                >
                  {busyEntity === e.id ? "…" : "Dispute"}
                </button>
              ) : (
                <div style={{ display: "flex", gap: 6, alignItems: "center" }}>
                  <input
                    style={{ ...styles.input, width: 260 }}
                    autoFocus
                    placeholder="Why is this disputed? (recorded for the audit trail)"
                    value={disputeReason}
                    onChange={(ev) => setDisputeReason(ev.target.value)}
                    onKeyDown={(ev) => ev.key === "Enter" && recordDispute(e.id)}
                  />
                  <button
                    style={{ ...styles.button, background: theme.redBg, color: theme.redText, ...(busyEntity !== null ? styles.disabled : {}) }}
                    disabled={busyEntity !== null}
                    onClick={() => recordDispute(e.id)}
                  >
                    {busyEntity === e.id ? "…" : "Record dispute"}
                  </button>
                  <button
                    style={styles.button}
                    onClick={() => {
                      setDisputeFor(null);
                      setDisputeReason("");
                    }}
                  >
                    Cancel
                  </button>
                </div>
              )}
            </div>
          </article>
        ))}
        {low.length === 0 && <p style={{ color: theme.textDim }}>Nothing needs review.</p>}
      </div>

      <h3 style={{ color: theme.textMuted }}>Possible duplicates</h3>
      <div style={{ display: "grid", gap: 8 }}>
        {dupes.map((d) => (
          <article key={d.id} style={{ background: theme.bgCard, border: `1px solid ${theme.border}`, borderRadius: 8, padding: "0.75rem 1rem" }}>
            <div style={{ fontSize: 13 }}>Entity #{d.entity_a_id} ↔ Entity #{d.entity_b_id}</div>
            <div style={{ fontSize: 12, color: theme.textMuted }}>{d.reason}</div>
            <div style={{ display: "flex", gap: 8, marginTop: 8 }}>
              <button
                style={{ ...styles.button, background: theme.greenBg, ...(busyProposal !== null ? styles.disabled : {}) }}
                disabled={busyProposal !== null}
                onClick={() => decide(d.id, "merge")}
              >
                {busyProposal === d.id ? "Merging…" : "Merge"}
              </button>
              <button
                style={{ ...styles.button, ...(busyProposal !== null ? styles.disabled : {}) }}
                disabled={busyProposal !== null}
                onClick={() => decide(d.id, "dismiss")}
              >
                {busyProposal === d.id ? "…" : "Not duplicates"}
              </button>
            </div>
          </article>
        ))}
        {dupes.length === 0 && <p style={{ color: theme.textDim }}>No duplicates found.</p>}
      </div>
    </div>
  );
}

const styles: Record<string, React.CSSProperties> = {
  button: {
    padding: "0.3rem 0.7rem",
    borderRadius: 6,
    border: `1px solid ${theme.border}`,
    background: theme.bgHover,
    color: theme.text,
    cursor: "pointer",
    fontSize: 12,
    opacity: 1,
  },
  disabled: { opacity: 0.5, cursor: "not-allowed" },
  input: {
    padding: "0.35rem 0.6rem",
    borderRadius: 6,
    border: `1px solid ${theme.border}`,
    background: theme.bgElevated,
    color: theme.text,
    fontSize: 12,
  },
  contextButton: {
    background: "none",
    border: "none",
    color: theme.accentAlt,
    cursor: "pointer",
    fontSize: 12,
    padding: 0,
    textDecoration: "underline",
  },
  contextBox: {
    marginTop: 6,
    padding: "0.6rem 0.75rem",
    background: theme.bgElevated,
    border: `1px solid ${theme.border}`,
    borderRadius: 6,
    maxHeight: 360,
    overflowY: "auto",
  },
  contextPre: {
    whiteSpace: "pre-wrap",
    margin: "6px 0",
    fontSize: 12,
    lineHeight: 1.5,
    color: theme.text,
    background: theme.bg,
    border: `1px solid ${theme.border}`,
    borderRadius: 6,
    padding: "0.5rem 0.6rem",
  },
  contextPreDim: {
    whiteSpace: "pre-wrap",
    margin: "6px 0",
    fontSize: 12,
    lineHeight: 1.5,
    color: theme.textDim,
    background: "transparent",
    border: `1px dashed ${theme.border}`,
    borderRadius: 6,
    padding: "0.5rem 0.6rem",
    opacity: 0.85,
  },
};
