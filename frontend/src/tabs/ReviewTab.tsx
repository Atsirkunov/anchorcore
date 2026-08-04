import { useCallback, useEffect, useState } from "react";
import { api } from "../api";
import type { Entity, MergeProposal } from "../types";

export function ReviewTab() {
  const [low, setLow] = useState<Entity[]>([]);
  const [dupes, setDupes] = useState<MergeProposal[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [busyEntity, setBusyEntity] = useState<number | null>(null);
  const [busyProposal, setBusyProposal] = useState<number | null>(null);

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

  return (
    <div>
      <h2>Review</h2>
      {error && <p style={{ color: "#f87171" }}>{error}</p>}

      <h3 style={{ color: "#9ca3af" }}>Low confidence — needs a human look</h3>
      <div style={{ display: "grid", gap: 8, marginBottom: 24 }}>
        {low.map((e) => (
          <article key={e.id} style={{ background: "#171a21", border: "1px solid #2d333b", borderRadius: 8, padding: "0.75rem 1rem" }}>
            <div style={{ fontSize: 12, color: "#9ca3af" }}>[{e.kind}] conf {(e.confidence * 100).toFixed(0)}% · {e.source_ref}</div>
            <p style={{ margin: "0.3rem 0", fontWeight: 600 }}>{e.summary}</p>
            <div style={{ display: "flex", gap: 8 }}>
              <button
                style={{ ...styles.button, ...(busyEntity !== null ? styles.disabled : {}) }}
                disabled={busyEntity !== null}
                onClick={() => updateEntity(e.id, { status: "verified" })}
              >
                {busyEntity === e.id ? "…" : "Looks right"}
              </button>
              <button
                style={{ ...styles.button, ...(busyEntity !== null ? styles.disabled : {}) }}
                disabled={busyEntity !== null}
                onClick={() => updateEntity(e.id, { status: "disputed" })}
              >
                {busyEntity === e.id ? "…" : "Dispute"}
              </button>
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
};
