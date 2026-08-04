import { useCallback, useEffect, useState } from "react";
import { api } from "../api";
import type { Entity } from "../types";

const KIND_COLORS: Record<string, string> = {
  decision: "#8b5cf6",
  document: "#3b82f6",
  action: "#f59e0b",
  note: "#6b7280",
};

export function EntitiesTab() {
  const [entities, setEntities] = useState<Entity[]>([]);
  const [kind, setKind] = useState<string>("");
  const [error, setError] = useState<string | null>(null);
  const [busyId, setBusyId] = useState<number | null>(null);

  const refresh = useCallback(() => {
    api
      .listEntities(kind ? { kind } : undefined)
      .then(setEntities)
      .catch((e) => setError(String(e)));
  }, [kind]);

  useEffect(() => refresh(), [refresh]);

  async function update(entity: Entity, patch: Partial<Entity>) {
    setBusyId(entity.id);
    setError(null);
    try {
      await api.updateEntity(entity.id, patch);
      refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusyId(null);
    }
  }

  return (
    <div>
      <div style={{ display: "flex", alignItems: "center", gap: 12, marginBottom: 12 }}>
        <h2 style={{ margin: 0 }}>Entities</h2>
        <select value={kind} onChange={(e) => setKind(e.target.value)} style={styles.input}>
          <option value="">All kinds</option>
          <option value="decision">decision</option>
          <option value="document">document</option>
          <option value="action">action</option>
          <option value="note">note</option>
        </select>
      </div>
      {error && <p style={{ color: "#f87171" }}>{error}</p>}
      <div style={{ display: "grid", gap: 8 }}>
        {entities.map((e) => (
          <article key={e.id} style={{ ...styles.card, borderLeft: `4px solid ${KIND_COLORS[e.kind] ?? "#6b7280"}` }}>
            <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
              <span style={{ ...styles.badge, background: KIND_COLORS[e.kind] ?? "#6b7280" }}>{e.kind}</span>
              <span style={{ ...styles.badge, background: e.status === "verified" ? "#14532d" : e.status === "stale" ? "#3f3f46" : "#451a03" }}>
                {e.status}
              </span>
              <span style={{ fontSize: 12, color: "#9ca3af" }}>conf {(e.confidence * 100).toFixed(0)}%</span>
              <span style={{ marginLeft: "auto", fontSize: 12, color: "#6b7280" }}>{e.source_ref}</span>
            </div>
            <p style={{ margin: "0.4rem 0", fontWeight: 600 }}>{e.summary}</p>
            {e.reasoning && <p style={{ margin: 0, color: "#9ca3af", fontSize: 13 }}>{e.reasoning}</p>}
            <div style={{ display: "flex", gap: 8, marginTop: 8 }}>
              <button style={{ ...styles.button, ...(busyId !== null ? styles.disabled : {}) }} disabled={busyId !== null} onClick={() => update(e, { status: "verified" })}>
                {busyId === e.id ? "…" : "Verify"}
              </button>
              <button style={{ ...styles.button, ...(busyId !== null ? styles.disabled : {}) }} disabled={busyId !== null} onClick={() => update(e, { status: "disputed" })}>
                {busyId === e.id ? "…" : "Dispute"}
              </button>
              <select
                value={e.kind}
                onChange={(ev) => update(e, { kind: ev.target.value as Entity["kind"] })}
                style={styles.input}
              >
                <option value="decision">decision</option>
                <option value="document">document</option>
                <option value="action">action</option>
                <option value="note">note</option>
              </select>
            </div>
          </article>
        ))}
        {entities.length === 0 && <p style={{ color: "#6b7280" }}>No entities yet. Ingest a source first.</p>}
      </div>
    </div>
  );
}

const styles: Record<string, React.CSSProperties> = {
  card: { background: "#171a21", border: "1px solid #2d333b", borderRadius: 8, padding: "0.75rem 1rem" },
  badge: { fontSize: 11, color: "#0f1115", padding: "0.1rem 0.5rem", borderRadius: 999, fontWeight: 700, textTransform: "uppercase" },
  button: { padding: "0.3rem 0.7rem", borderRadius: 6, border: "1px solid #2d333b", background: "#1e2430", color: "#e6e8eb", cursor: "pointer", fontSize: 12 },
  disabled: { opacity: 0.5, cursor: "not-allowed" },
  input: { padding: "0.3rem 0.5rem", borderRadius: 6, border: "1px solid #2d333b", background: "#171a21", color: "#e6e8eb", fontSize: 12 },
};
