import { useCallback, useEffect, useState } from "react";
import { api } from "../api";
import type { Entity, MergeProposal } from "../types";

export function ReviewTab() {
  const [low, setLow] = useState<Entity[]>([]);
  const [dupes, setDupes] = useState<MergeProposal[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [busyEntity, setBusyEntity] = useState<number | null>(null);
  const [busyProposal, setBusyProposal] = useState<number | null>(null);
  const [expanded, setExpanded] = useState<number | null>(null);
  const [disputeFor, setDisputeFor] = useState<number | null>(null);
  const [disputeReason, setDisputeReason] = useState("");

  const refresh = useCallback(() => {
    api.lowConfidence().then(setLow).catch((e) => setError(String(e)));
    api.duplicates().then(setDupes).catch((e) => setError(String(e)));
  }, []);

  useEffect(() => refresh(), [refresh]);

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
      {error && <p style={{ color: "#f87171" }}>{error}</p>}

      <h3 style={{ color: "#9ca3af" }}>Low confidence — needs a human look</h3>
      <div style={{ display: "grid", gap: 8, marginBottom: 24 }}>
        {low.map((e) => (
          <article key={e.id} style={{ background: "#171a21", border: "1px solid #2d333b", borderRadius: 8, padding: "0.75rem 1rem" }}>
            <div style={{ fontSize: 12, color: "#9ca3af" }}>
              [{e.kind}] conf {(e.confidence * 100).toFixed(0)}% · {e.source_ref}
              {e.dispute_count > 0 && (
                <span title={`Disputed ${e.dispute_count}× — excluded from Q&A`} style={{ marginLeft: 6, color: "#fca5a5" }}>
                  ⚑ disputed ×{e.dispute_count}
                </span>
              )}
            </div>
            <p style={{ margin: "0.3rem 0", fontWeight: 600 }}>{e.summary}</p>
            {e.reasoning && <p style={{ margin: 0, color: "#9ca3af", fontSize: 13 }}>{e.reasoning}</p>}
            {e.window_text && (
              <div style={{ marginTop: 8 }}>
                <button
                  onClick={() => setExpanded(expanded === e.id ? null : e.id)}
                  style={styles.contextButton}
                >
                  {expanded === e.id ? "Hide classifier context ▲" : "Show what the classifier saw ▼"}
                </button>
                {expanded === e.id && (
                  <div style={styles.contextBox}>
                    <div style={{ fontSize: 11, color: "#6b7280", marginBottom: 4, textTransform: "uppercase" }}>
                      Classifier input window (source excerpt)
                    </div>
                    <pre style={{ whiteSpace: "pre-wrap", margin: 0, fontSize: 12, lineHeight: 1.5, color: "#d1d5db" }}>{e.window_text}</pre>
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
                    style={{ ...styles.button, background: "#3a1d1d", color: "#fca5a5", ...(busyEntity !== null ? styles.disabled : {}) }}
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
        {low.length === 0 && <p style={{ color: "#6b7280" }}>Nothing needs review.</p>}
      </div>

      <h3 style={{ color: "#9ca3af" }}>Possible duplicates</h3>
      <div style={{ display: "grid", gap: 8 }}>
        {dupes.map((d) => (
          <article key={d.id} style={{ background: "#171a21", border: "1px solid #2d333b", borderRadius: 8, padding: "0.75rem 1rem" }}>
            <div style={{ fontSize: 13 }}>Entity #{d.entity_a_id} ↔ Entity #{d.entity_b_id}</div>
            <div style={{ fontSize: 12, color: "#9ca3af" }}>{d.reason}</div>
            <div style={{ display: "flex", gap: 8, marginTop: 8 }}>
              <button
                style={{ ...styles.button, background: "#1e3a2a", ...(busyProposal !== null ? styles.disabled : {}) }}
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
        {dupes.length === 0 && <p style={{ color: "#6b7280" }}>No duplicates found.</p>}
      </div>
    </div>
  );
}

const styles: Record<string, React.CSSProperties> = {
  button: {
    padding: "0.3rem 0.7rem",
    borderRadius: 6,
    border: "1px solid #2d333b",
    background: "#1e2430",
    color: "#e6e8eb",
    cursor: "pointer",
    fontSize: 12,
    opacity: 1,
  },
  disabled: { opacity: 0.5, cursor: "not-allowed" },
  input: {
    padding: "0.35rem 0.6rem",
    borderRadius: 6,
    border: "1px solid #2d333b",
    background: "#14171d",
    color: "#e6e8eb",
    fontSize: 12,
  },
  contextButton: {
    background: "none",
    border: "none",
    color: "#7dd3fc",
    cursor: "pointer",
    fontSize: 12,
    padding: 0,
    textDecoration: "underline",
  },
  contextBox: {
    marginTop: 6,
    padding: "0.6rem 0.75rem",
    background: "#14171d",
    border: "1px solid #2d333b",
    borderRadius: 6,
    maxHeight: 300,
    overflowY: "auto",
  },
};
