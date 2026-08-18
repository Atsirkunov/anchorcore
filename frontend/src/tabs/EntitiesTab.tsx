import { useCallback, useEffect, useState } from "react";
import { api } from "../api";
import { theme } from "../theme";
import type { Entity } from "../types";

const KIND_COLORS: Record<string, string> = {
  decision: theme.purple,
  document: "#3b82f6",
  action: "#f59e0b",
  note: "#6b7280",
};

export function EntitiesTab() {
  const [entities, setEntities] = useState<Entity[]>([]);
  const [kind, setKind] = useState<string>("");
  const [error, setError] = useState<string | null>(null);
  const [busyId, setBusyId] = useState<number | null>(null);
  const [expanded, setExpanded] = useState<number | null>(null);
  const [ctx, setCtx] = useState<Record<number, { source_name: string | null; source_ref: string; window_text: string; expanded_before: string[]; expanded_after: string[]; highlight: string; item_title: string }>>({});

  const refresh = useCallback(() => {
    api
      .listEntities(kind ? { kind } : undefined)
      .then(setEntities)
      .catch((e) => setError(String(e)));
  }, [kind]);

  useEffect(() => refresh(), [refresh]);

  async function toggleCtx(e: Entity) {
    const next = expanded === e.id ? null : e.id;
    setExpanded(next);
    if (next !== null && !ctx[e.id] && e.window_text) {
      try {
        const c = await api.entityContext(e.id);
        setCtx((m) => ({ ...m, [e.id]: c }));
      } catch {}
    }
  }

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
      {error && <p style={{ color: theme.red }}>{error}</p>}
      <div style={{ display: "grid", gap: 8 }}>
        {entities.map((e) => (
          <article key={e.id} style={{ ...styles.card, borderLeft: `4px solid ${KIND_COLORS[e.kind] ?? "#6b7280"}` }}>
            <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
              <span style={{ ...styles.badge, background: KIND_COLORS[e.kind] ?? "#6b7280" }}>{e.kind}</span>
              <span style={{ ...styles.badge, background: e.status === "verified" ? "#14532d" : e.status === "stale" ? "#3f3f46" : "#451a03" }}>
                {e.status}
              </span>
              <span style={{ fontSize: 12, color: theme.textMuted }}>conf {(e.confidence * 100).toFixed(0)}%</span>
              <span style={{ marginLeft: "auto", fontSize: 12, color: theme.textDim }}>{e.source_ref}</span>
            </div>
            <p style={{ margin: "0.4rem 0", fontWeight: 600 }}>{e.summary}</p>
            {e.reasoning && <p style={{ margin: 0, color: theme.textMuted, fontSize: 13 }}>{e.reasoning}</p>}
            {e.window_text && (
              <div style={{ marginTop: 6 }}>
                <button onClick={() => toggleCtx(e)} style={{ background: "none", border: "none", color: theme.accentAlt, cursor: "pointer", fontSize: 12, padding: 0, textDecoration: "underline" }}>
                  {expanded === e.id ? "Hide context ▲" : "Show context ▼"}
                </button>
                {expanded === e.id && (
                  <div style={{ marginTop: 6, padding: "0.6rem 0.75rem", background: theme.bgElevated, border: `1px solid ${theme.border}`, borderRadius: 6, maxHeight: 360, overflowY: "auto" }}>
                    <div style={{ fontSize: 11, color: theme.textDim, textTransform: "uppercase", marginBottom: 4 }}>
                      {ctx[e.id]?.item_title || "Source"} · {ctx[e.id]?.source_ref || e.source_ref}
                    </div>
                    {ctx[e.id]?.expanded_before.map((t, i) => (
                      <pre key={i} style={{ whiteSpace: "pre-wrap", margin: "6px 0", fontSize: 12, color: theme.textDim, opacity: 0.85 }}>{t}</pre>
                    ))}
                    <pre style={{ whiteSpace: "pre-wrap", margin: "6px 0", fontSize: 12, background: theme.bg, border: `1px solid ${theme.border}`, borderRadius: 6, padding: "0.5rem 0.6rem", color: "#d1d5db" }}>
                      {ctx[e.id]?.window_text || e.window_text}
                    </pre>
                    {ctx[e.id]?.expanded_after.map((t, i) => (
                      <pre key={i} style={{ whiteSpace: "pre-wrap", margin: "6px 0", fontSize: 12, color: theme.textDim, opacity: 0.85 }}>{t}</pre>
                    ))}
                  </div>
                )}
              </div>
            )}
            <div style={{ display: "flex", gap: 8, marginTop: 8 }}>
              <button style={{ ...styles.button, ...(busyId === e.id ? styles.disabled : {}) }} disabled={busyId === e.id} onClick={() => update(e, { status: "verified" })}>
                {busyId === e.id ? "…" : "Verify"}
              </button>
              <button style={{ ...styles.button, ...(busyId === e.id ? styles.disabled : {}) }} disabled={busyId === e.id} onClick={() => update(e, { status: "disputed" })}>
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
        {entities.length === 0 && <p style={{ color: theme.textDim }}>No entities yet. Ingest a source first.</p>}
      </div>
    </div>
  );
}

const styles: Record<string, React.CSSProperties> = {
  card: { background: theme.bgCard, border: `1px solid ${theme.border}`, borderRadius: 8, padding: "0.75rem 1rem" },
  badge: { fontSize: 11, color: theme.bg, padding: "0.1rem 0.5rem", borderRadius: 999, fontWeight: 700, textTransform: "uppercase" },
  button: { padding: "0.3rem 0.7rem", borderRadius: 6, border: `1px solid ${theme.border}`, background: theme.bgHover, color: theme.text, cursor: "pointer", fontSize: 12 },
  disabled: { opacity: 0.5, cursor: "not-allowed" },
  input: { padding: "0.3rem 0.5rem", borderRadius: 6, border: `1px solid ${theme.border}`, background: theme.bgCard, color: theme.text, fontSize: 12 },
};
