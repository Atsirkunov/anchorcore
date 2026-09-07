import { useCallback, useEffect, useMemo, useState } from "react";
import { api } from "../api";
import { theme } from "../theme";
import type { Entity, Section, Tag } from "../types";

function Breadcrumb({ path, onSelect }: { path: string; onSelect?: (part: string) => void }) {
  if (!path) return null;
  const parts = path.split(" > ");
  return (
    <span style={{ fontSize: 11, color: theme.textDim }}>
      {parts.map((p, i) => (
        <span key={i}>
          {i > 0 && <span style={{ margin: "0 4px", color: theme.textMuted }}>›</span>}
          <span
            onClick={() => onSelect?.(p)}
            style={{ cursor: onSelect ? "pointer" : "default", textDecoration: onSelect ? "underline" : "none", color: i === parts.length - 1 ? theme.text : theme.textDim }}
            title={p}
          >
            {p}
          </span>
        </span>
      ))}
    </span>
  );
}

function TagChips({ names }: { names: string[] }) {
  if (names.length === 0) return null;
  return (
    <span style={{ display: "inline-flex", gap: 4, flexWrap: "wrap" }}>
      {names.slice(0, 4).map((n) => (
        <span key={n} style={{ fontSize: 10, background: theme.bgHover, color: theme.accentAlt, border: `1px solid ${theme.border}`, padding: "0.1rem 0.4rem", borderRadius: 999 }}>{n}</span>
      ))}
    </span>
  );
}

const KIND_COLORS: Record<string, string> = {
  decision: theme.purple,
  document: theme.blue,
  action: theme.amber,
  note: theme.textDim,
};

export function EntitiesTab() {
  const [entities, setEntities] = useState<Entity[]>([]);
  const [kind, setKind] = useState<string>("");
  const [statusFilter, setStatusFilter] = useState<string>("needs_review");
  const [error, setError] = useState<string | null>(null);
  const [busyId, setBusyId] = useState<number | null>(null);
  const [expanded, setExpanded] = useState<number | null>(null);
  const [ctx, setCtx] = useState<Record<number, { source_name: string | null; source_ref: string; window_text: string; expanded_before: string[]; expanded_after: string[]; highlight: string; item_title: string; full_text?: string }>>({});
  // grouped view: Map<item_id, Entity[]> — zero backend change, client-side only
  const [groupedPref, setGroupedPref] = useState<boolean | null>(null);
  const isGrouped = groupedPref ?? entities.length > 200;
  const [expandedDocs, setExpandedDocs] = useState<Set<number>>(new Set());
  const [docCtx, setDocCtx] = useState<Record<number, { item_title: string; source_ref: string; full_text: string; source_name: string | null }>>({});
  const [piiByItem, setPiiByItem] = useState<Record<number, { is_pii: boolean; flagged: number; categories: string[]; matches: { category: string; label: string; match: string }[] }>>({});
  const [revealedDocs, setRevealedDocs] = useState<Set<number>>(new Set());
  const [verifiedDocs, setVerifiedDocs] = useState<Set<number>>(new Set());
  const [sectionsByItem, setSectionsByItem] = useState<Record<number, Section[]>>({});
  const [allTags, setAllTags] = useState<Tag[]>([]);

  useEffect(() => {
    api.tags().then(setAllTags).catch(() => {});
  }, []);

  const refresh = useCallback(() => {
    const statusParam = statusFilter && statusFilter !== "needs_review" ? statusFilter : undefined;
    api
      .listEntities({ kind: kind || undefined, status: statusParam, limit: 2000 })
      .then(setEntities)
      .catch((e) => setError(String(e)));
  }, [kind, statusFilter]);

  useEffect(() => refresh(), [refresh]);

  async function toggleCtx(e: Entity) {
    const next = expanded === e.id ? null : e.id;
    setExpanded(next);
    if (next !== null && !ctx[e.id] && e.window_text) {
      try {
        const c = await api.entityContext(e.id);
        setCtx((m) => ({ ...m, [e.id]: c }));
      } catch { void 0; }
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

  const filtered = useMemo(() => {
    return entities.filter((e) => {
      if (kind && e.kind !== kind) return false;
      if (statusFilter === "needs_review") return e.status === "unverified" || e.status === "disputed";
      if (statusFilter && e.status !== statusFilter) return false;
      return true;
    });
  }, [entities, kind, statusFilter]);

  const groups = useMemo(() => {
    const m = new Map<number, Entity[]>();
    for (const e of filtered) {
      const key = e.item_id ?? -1;
      const arr = m.get(key);
      if (arr) arr.push(e);
      else m.set(key, [e]);
    }
    // priority: docs with most unverified first, then largest, then id
    return Array.from(m.entries()).sort((a, b) => {
      const aUnverified = a[1].filter((e) => e.status === "unverified" || e.status === "disputed").length;
      const bUnverified = b[1].filter((e) => e.status === "unverified" || e.status === "disputed").length;
      if (bUnverified !== aUnverified) return bUnverified - aUnverified;
      return b[1].length - a[1].length || a[0] - b[0];
    });
  }, [filtered]);

  // eagerly fetch PII badge for first 30 docs (avoids 200+ requests)
  useEffect(() => {
    if (groups.length === 0) return;
    const toFetch = groups.slice(0, 30).filter(([id]) => piiByItem[id] == null);
    for (const [itemId] of toFetch) {
      api.itemPii(itemId).then((p) => setPiiByItem((m) => ({ ...m, [itemId]: p }))).catch(() => {});
    }
    // also prefetch sections for grouped TOC chips
    const secFetch = groups.slice(0, 30).filter(([id]) => sectionsByItem[id] == null);
    for (const [itemId] of secFetch) {
      api.sections({ item_id: itemId }).then((secs) => setSectionsByItem((m) => ({ ...m, [itemId]: secs }))).catch(() => {});
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [groups]);

  // flat view: also fetch sections for first 30 entities by item_id
  useEffect(() => {
    if (isGrouped) return;
    const ids = Array.from(new Set(filtered.slice(0, 30).map((e) => e.item_id).filter((v): v is number => v != null)));
    for (const itemId of ids) {
      if (sectionsByItem[itemId] == null) {
        api.sections({ item_id: itemId }).then((secs) => setSectionsByItem((m) => ({ ...m, [itemId]: secs }))).catch(() => {});
      }
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [filtered, isGrouped]);

  function maskPII(text: string, matches: { match: string }[]): string {
    let out = text;
    for (const m of matches) {
      if (!m.match) continue;
      const esc = m.match.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
      try {
        const re = new RegExp(esc, "gi");
        out = out.replace(re, "••••");
      } catch { void 0; }
    }
    return out;
  }

  async function toggleDoc(itemId: number, firstEntityId: number) {
    const next = new Set(expandedDocs);
    const isOpen = next.has(itemId);
    if (isOpen) next.delete(itemId);
    else next.add(itemId);
    setExpandedDocs(next);
    if (!isOpen) {
      if (docCtx[itemId] == null) {
        try {
          const c = await api.entityContext(firstEntityId);
          setDocCtx((m) => ({ ...m, [itemId]: { item_title: c.item_title, source_ref: c.source_ref, full_text: (c as unknown as { full_text?: string }).full_text ?? c.window_text, source_name: c.source_name } }));
        } catch { void 0; }
      }
      if (piiByItem[itemId] == null) {
        try {
          const p = await api.itemPii(itemId);
          setPiiByItem((m) => ({ ...m, [itemId]: p }));
        } catch { void 0; }
      }
      if (sectionsByItem[itemId] == null) {
        try {
          const secs = await api.sections({ item_id: itemId });
          setSectionsByItem((m) => ({ ...m, [itemId]: secs }));
        } catch { void 0; }
      }
    }
  }

  async function bulkUpdate(itemId: number, patch: Partial<Entity>) {
    const list = groups.find(([k]) => k === itemId)?.[1] ?? [];
    for (const e of list) {
      await update(e, patch);
    }
    if (patch.status === "verified") {
      // close all disputes: mark verified + collapse into navigable doc
      setVerifiedDocs((s) => new Set(s).add(itemId));
      // auto-expand doc to show whole doc view, hide session entities
      setExpandedDocs((s) => new Set(s).add(itemId));
      // ensure doc text loaded
      const first = list[0];
      if (first && docCtx[itemId] == null) {
        try {
          const c = await api.entityContext(first.id);
          setDocCtx((m) => ({ ...m, [itemId]: { item_title: c.item_title, source_ref: c.source_ref, full_text: (c as unknown as { full_text?: string }).full_text ?? c.window_text, source_name: c.source_name } }));
        } catch { void 0; }
      }
    } else if (patch.status === "disputed") {
      setVerifiedDocs((s) => {
        const n = new Set(s);
        n.delete(itemId);
        return n;
      });
    }
  }

  return (
    <div>
      <div style={{ display: "flex", alignItems: "center", gap: 12, marginBottom: 12, flexWrap: "wrap" }}>
        <h2 style={{ margin: 0 }}>Entities</h2>
        <select value={kind} onChange={(e) => setKind(e.target.value)} style={styles.input}>
          <option value="">All kinds</option>
          <option value="decision">decision</option>
          <option value="document">document</option>
          <option value="action">action</option>
          <option value="note">note</option>
        </select>
        <select value={statusFilter} onChange={(e) => setStatusFilter(e.target.value)} style={styles.input}>
          <option value="needs_review">⚠ Needs review</option>
          <option value="">All statuses</option>
          <option value="unverified">Unverified</option>
          <option value="verified">Verified</option>
          <option value="disputed">Disputed</option>
          <option value="stale">Stale</option>
        </select>
        <span style={{ fontSize: 12, color: statusFilter === "needs_review" ? theme.amber : theme.textDim }}>
          {filtered.length} / {entities.length} entities{isGrouped ? ` · ${groups.length} docs` : ""} {statusFilter === "needs_review" ? "· flagged" : ""}
        </span>
        <div style={{ marginLeft: "auto", display: "flex", gap: 4, alignItems: "center", border: `1px solid ${theme.border}`, borderRadius: 999, padding: 2, background: theme.bgCard }}>
          <button onClick={() => setGroupedPref(true)} style={{ ...styles.toggle, ...(isGrouped ? styles.toggleActive : {}) }}>Grouped</button>
          <button onClick={() => setGroupedPref(false)} style={{ ...styles.toggle, ...(!isGrouped ? styles.toggleActive : {}) }}>Flat</button>
        </div>
      </div>
      {error && <p style={{ color: theme.red }}>{error}</p>}
      {isGrouped ? (
        <div style={{ display: "grid", gap: 10 }}>
          {groups.map(([itemId, list]) => {
            const first = list[0];
            const avgConf = list.reduce((s, e) => s + e.confidence, 0) / list.length;
            const isOpen = expandedDocs.has(itemId);
            const docTitle = docCtx[itemId]?.item_title || first.source_ref.split("/").pop() || `Doc #${itemId}`;
            const docRef = docCtx[itemId]?.source_ref || first.source_ref;
            const pii = piiByItem[itemId];
            const isPii = !!pii?.is_pii;
            const isRevealed = revealedDocs.has(itemId);
            const isDocVerified = verifiedDocs.has(itemId) || list.every((e) => e.status === "verified");
            return (
              <section key={itemId} style={{ ...styles.docCard, borderLeft: `4px solid ${isDocVerified ? theme.greenBg : theme.accent}` }}>
                <div style={{ display: "flex", gap: 10, alignItems: "center", flexWrap: "wrap" }}>
                  <div style={{ flex: 1, minWidth: 220 }}>
                    <div style={{ fontWeight: 700, fontSize: 13, color: theme.text, whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>{docTitle}</div>
                    <div style={{ fontSize: 11, color: theme.textDim, whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>{docRef} · item #{itemId}</div>
                    {sectionsByItem[itemId] && sectionsByItem[itemId].length > 0 && (
                      <div style={{ marginTop: 4, display: "flex", gap: 8, alignItems: "center", flexWrap: "wrap" }}>
                        <Breadcrumb path={sectionsByItem[itemId][0].path} />
                        <TagChips
                          names={allTags
                            .filter((t) => sectionsByItem[itemId].some((s) => s.path.toLowerCase().includes(t.name) || s.title.toLowerCase().includes(t.name)))
                            .slice(0, 4)
                            .map((t) => t.name)}
                        />
                      </div>
                    )}
                  </div>
                  <span style={styles.docCount}>{list.length} entities</span>
                  {(() => {
                    const need = list.filter((e) => e.status === "unverified" || e.status === "disputed").length;
                    return need > 0 ? <span style={styles.needBadge}>⚠ {need} need review</span> : null;
                  })()}
                  <span style={{ fontSize: 11, color: theme.textMuted }}>avg {(avgConf * 100).toFixed(0)}%</span>
                  <span style={{ display: "flex", gap: 4 }}>
                    {Array.from(new Set(list.map((e) => e.kind))).map((k) => (
                      <span key={k} title={k} style={{ width: 8, height: 8, borderRadius: 999, background: KIND_COLORS[k as string] ?? theme.textDim, display: "inline-block", border: `1px solid ${theme.border}` }} />
                    ))}
                  </span>
                  {isPii && (
                    <span title={(pii?.categories ?? []).join(", ")} style={styles.piiBadge}>
                      PII ● {pii?.flagged ?? pii?.categories.length}
                    </span>
                  )}
                  {isDocVerified && <span style={{ fontSize: 10, background: theme.greenBg, color: theme.greenText, padding: "0.15rem 0.5rem", borderRadius: 999, fontWeight: 700 }}>✓ Verified</span>}
                  <button onClick={() => toggleDoc(itemId, first.id)} style={styles.docToggle}>
                    {isOpen ? "Hide doc ▲" : isDocVerified ? "Open doc ▶" : "Show doc ▼"}
                  </button>
                </div>
                {isOpen && docCtx[itemId]?.full_text && (
                  <div style={{ marginTop: 8, padding: "0.6rem 0.75rem", background: theme.bgElevated, border: `1px solid ${theme.border}`, borderRadius: 6, maxHeight: 320, overflowY: "auto" }}>
                    <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: 4 }}>
                      <div style={{ fontSize: 11, color: theme.textDim, textTransform: "uppercase" }}>Whole doc — {docTitle}</div>
                      {isPii && (
                        <button
                          onClick={() => {
                            const next = new Set(revealedDocs);
                            if (isRevealed) next.delete(itemId);
                            else next.add(itemId);
                            setRevealedDocs(next);
                          }}
                          style={{ ...styles.docToggle, borderColor: theme.amber, color: theme.amber }}
                        >
                          {isRevealed ? "Hide PII" : "Reveal PII"}
                        </button>
                      )}
                    </div>
                    {sectionsByItem[itemId] && sectionsByItem[itemId].length > 0 && (
                      <div style={{ marginBottom: 8, display: "grid", gap: 3, padding: "0.4rem 0.5rem", background: theme.bg, border: `1px solid ${theme.border}`, borderRadius: 6 }}>
                        <div style={{ fontSize: 10, color: theme.textDim, textTransform: "uppercase" }}>Sections — TOC</div>
                        {sectionsByItem[itemId].map((s) => (
                          <div key={s.id} style={{ display: "flex", gap: 6, alignItems: "center", fontSize: 11 }}>
                            <Breadcrumb path={s.path || s.title} />
                            <span style={{ color: theme.textMuted, fontSize: 10 }}>lvl {s.level}</span>
                          </div>
                        ))}
                      </div>
                    )}
                    {isPii && !isRevealed && <div style={{ fontSize: 11, color: theme.amber, marginBottom: 6 }}>PII hidden — {pii?.categories.join(", ")} · click Reveal to show</div>}
                    <pre style={{ whiteSpace: "pre-wrap", margin: 0, fontSize: 12, color: theme.textMuted, lineHeight: 1.5 }}>
                      {isPii && !isRevealed
                        ? maskPII(docCtx[itemId].full_text.slice(0, 16000), pii?.matches ?? [])
                        : docCtx[itemId].full_text.slice(0, 16000)}
                    </pre>
                  </div>
                )}
                {isDocVerified ? (
                  <div style={{ marginTop: 8, padding: "0.5rem 0.75rem", background: theme.greenBg, border: `1px solid ${theme.greenBg}`, borderRadius: 6, display: "flex", alignItems: "center", gap: 8 }}>
                    <span style={{ fontSize: 12, color: theme.onAccent, fontWeight: 700 }}>✓ Verified</span>
                    <span style={{ fontSize: 12, color: theme.greenText }}>{list.length} entities verified — doc collapsed, navigate via whole doc above</span>
                    <button onClick={() => toggleDoc(itemId, first.id)} style={{ marginLeft: "auto", ...styles.docToggle, borderColor: theme.greenText, color: theme.greenText }}>{isOpen ? "Hide" : "Open doc"}</button>
                    <button
                      onClick={() => {
                        setVerifiedDocs((s) => {
                          const n = new Set(s);
                          n.delete(itemId);
                          return n;
                        });
                      }}
                      style={styles.docToggle}
                    >
                      Reopen session
                    </button>
                  </div>
                ) : (
                  <div style={{ display: "flex", gap: 6, marginTop: 8 }}>
                    <button onClick={() => bulkUpdate(itemId, { status: "verified" })} style={{ ...styles.button, background: theme.accent, color: theme.onAccent }}>
                      Verify all & collapse
                    </button>
                    <button onClick={() => bulkUpdate(itemId, { status: "disputed" })} style={styles.button}>
                      Dispute doc
                    </button>
                  </div>
                )}
                {!isDocVerified && (
                  <div style={{ display: "grid", gap: 6, marginTop: 10 }}>
                    {list.map((e) => (
                      <article key={e.id} style={{ ...styles.cardSmall, borderLeft: `3px solid ${KIND_COLORS[e.kind] ?? theme.textDim}` }}>
                        <div style={{ display: "flex", gap: 6, alignItems: "center" }}>
                          <span style={{ ...styles.badge, background: KIND_COLORS[e.kind] ?? theme.textDim, fontSize: 10 }}>{e.kind}</span>
                          <span style={{ ...styles.badge, background: e.status === "verified" ? theme.greenBg : e.status === "stale" ? theme.mutedBg : theme.amberBg, fontSize: 10 }}>{e.status}</span>
                        <span style={{ fontSize: 11, color: theme.textMuted }}>conf {(e.confidence * 100).toFixed(0)}%</span>
                        <span style={{ marginLeft: "auto", fontSize: 11, color: theme.textDim, maxWidth: 180, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{e.source_ref}</span>
                      </div>
                      <p style={{ margin: "0.3rem 0", fontWeight: 600, fontSize: 13 }}>{e.summary}</p>
                      {e.reasoning && <p style={{ margin: 0, color: theme.textMuted, fontSize: 12 }}>{e.reasoning}</p>}
                      {e.window_text && (
                        <div style={{ marginTop: 4 }}>
                          <button onClick={() => toggleCtx(e)} style={{ background: "none", border: "none", color: theme.accentAlt, cursor: "pointer", fontSize: 11, padding: 0, textDecoration: "underline" }}>
                            {expanded === e.id ? "Hide context ▲" : "Show context ▼"}
                          </button>
                          {expanded === e.id && (
                            <div style={{ marginTop: 4, padding: "0.5rem 0.6rem", background: theme.bgElevated, border: `1px solid ${theme.border}`, borderRadius: 6, maxHeight: 300, overflowY: "auto" }}>
                              <div style={{ fontSize: 10, color: theme.textDim, textTransform: "uppercase", marginBottom: 4 }}>{ctx[e.id]?.item_title || "Source"} · {ctx[e.id]?.source_ref || e.source_ref}</div>
                              {ctx[e.id]?.expanded_before.map((t, i) => (
                                <pre key={i} style={{ whiteSpace: "pre-wrap", margin: "4px 0", fontSize: 11, color: theme.textDim, opacity: 0.85 }}>{t}</pre>
                              ))}
                              <pre style={{ whiteSpace: "pre-wrap", margin: "4px 0", fontSize: 11, background: theme.bg, border: `1px solid ${theme.border}`, borderRadius: 6, padding: "0.4rem 0.5rem", color: theme.text }}>{ctx[e.id]?.window_text || e.window_text}</pre>
                              {ctx[e.id]?.expanded_after.map((t, i) => (
                                <pre key={i} style={{ whiteSpace: "pre-wrap", margin: "4px 0", fontSize: 11, color: theme.textDim, opacity: 0.85 }}>{t}</pre>
                              ))}
                            </div>
                          )}
                        </div>
                      )}
                      <div style={{ display: "flex", gap: 6, marginTop: 6 }}>
                        <button style={{ ...styles.button, ...(busyId === e.id ? styles.disabled : {}) }} disabled={busyId === e.id} onClick={() => update(e, { status: "verified" })}>{busyId === e.id ? "…" : "Verify"}</button>
                        <button style={{ ...styles.button, ...(busyId === e.id ? styles.disabled : {}) }} disabled={busyId === e.id} onClick={() => update(e, { status: "disputed" })}>{busyId === e.id ? "…" : "Dispute"}</button>
                      </div>
                    </article>
                  ))}
                </div>
                )}
              </section>
            );
          })}
          {groups.length === 0 && <p style={{ color: theme.textDim }}>No entities yet. Ingest a source first.</p>}
        </div>
      ) : (
        <div style={{ display: "grid", gap: 8 }}>
          {filtered.map((e) => (
            <article key={e.id} style={{ ...styles.card, borderLeft: `4px solid ${KIND_COLORS[e.kind] ?? theme.textDim}` }}>
              <div style={{ display: "flex", gap: 8, alignItems: "center", flexWrap: "wrap" }}>
                <span style={{ ...styles.badge, background: KIND_COLORS[e.kind] ?? theme.textDim }}>{e.kind}</span>
                <span style={{ ...styles.badge, background: e.status === "verified" ? theme.greenBg : e.status === "stale" ? theme.mutedBg : theme.amberBg }}>{e.status}</span>
                <span style={{ fontSize: 12, color: theme.textMuted }}>conf {(e.confidence * 100).toFixed(0)}%</span>
                <span style={{ marginLeft: "auto", fontSize: 12, color: theme.textDim }}>{e.source_ref}</span>
              </div>
              {e.item_id != null && sectionsByItem[e.item_id] && sectionsByItem[e.item_id].length > 0 && (
                <div style={{ marginTop: 4, display: "flex", gap: 8, alignItems: "center", flexWrap: "wrap" }}>
                  <Breadcrumb path={sectionsByItem[e.item_id][0].path} />
                  <TagChips
                    names={allTags
                      .filter((t) => sectionsByItem[e.item_id]!.some((s) => s.path.toLowerCase().includes(t.name) || s.title.toLowerCase().includes(t.name)))
                      .slice(0, 4)
                      .map((t) => t.name)}
                  />
                </div>
              )}
              <p style={{ margin: "0.4rem 0", fontWeight: 600 }}>{e.summary}</p>
              {e.reasoning && <p style={{ margin: 0, color: theme.textMuted, fontSize: 13 }}>{e.reasoning}</p>}
              {e.window_text && (
                <div style={{ marginTop: 6 }}>
                  <button onClick={() => toggleCtx(e)} style={{ background: "none", border: "none", color: theme.accentAlt, cursor: "pointer", fontSize: 12, padding: 0, textDecoration: "underline" }}>
                    {expanded === e.id ? "Hide context ▲" : "Show context ▼"}
                  </button>
                  {expanded === e.id && (
                    <div style={{ marginTop: 6, padding: "0.6rem 0.75rem", background: theme.bgElevated, border: `1px solid ${theme.border}`, borderRadius: 6, maxHeight: 360, overflowY: "auto" }}>
                      <div style={{ fontSize: 11, color: theme.textDim, textTransform: "uppercase", marginBottom: 4 }}>{ctx[e.id]?.item_title || "Source"} · {ctx[e.id]?.source_ref || e.source_ref}</div>
                      {ctx[e.id]?.expanded_before.map((t, i) => (
                        <pre key={i} style={{ whiteSpace: "pre-wrap", margin: "6px 0", fontSize: 12, color: theme.textDim, opacity: 0.85 }}>{t}</pre>
                      ))}
                      <pre style={{ whiteSpace: "pre-wrap", margin: "6px 0", fontSize: 12, background: theme.bg, border: `1px solid ${theme.border}`, borderRadius: 6, padding: "0.5rem 0.6rem", color: theme.text }}>{ctx[e.id]?.window_text || e.window_text}</pre>
                      {ctx[e.id]?.expanded_after.map((t, i) => (
                        <pre key={i} style={{ whiteSpace: "pre-wrap", margin: "6px 0", fontSize: 12, color: theme.textDim, opacity: 0.85 }}>{t}</pre>
                      ))}
                    </div>
                  )}
                </div>
              )}
              <div style={{ display: "flex", gap: 8, marginTop: 8 }}>
                <button style={{ ...styles.button, ...(busyId === e.id ? styles.disabled : {}) }} disabled={busyId === e.id} onClick={() => update(e, { status: "verified" })}>{busyId === e.id ? "…" : "Verify"}</button>
                <button style={{ ...styles.button, ...(busyId === e.id ? styles.disabled : {}) }} disabled={busyId === e.id} onClick={() => update(e, { status: "disputed" })}>{busyId === e.id ? "…" : "Dispute"}</button>
                <select value={e.kind} onChange={(ev) => update(e, { kind: ev.target.value as Entity["kind"] })} style={styles.input}>
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
      )}
    </div>
  );
}

const styles: Record<string, React.CSSProperties> = {
  card: { background: theme.bgCard, border: `1px solid ${theme.border}`, borderRadius: 8, padding: "0.75rem 1rem" },
  cardSmall: { background: theme.bgElevated, border: `1px solid ${theme.border}`, borderRadius: 7, padding: "0.6rem 0.8rem" },
  docCard: { background: theme.bgCard, border: `1px solid ${theme.border}`, borderRadius: 8, padding: "0.7rem 0.9rem" },
  docCount: { fontSize: 11, background: theme.bgHover, color: theme.textMuted, padding: "0.15rem 0.5rem", borderRadius: 999, fontWeight: 600 },
  needBadge: { fontSize: 10, background: theme.amberBg, color: theme.amber, border: `1px solid ${theme.amber}`, padding: "0.15rem 0.5rem", borderRadius: 999, fontWeight: 700 },
  docToggle: { background: "none", border: `1px solid ${theme.border}`, color: theme.accentAlt, borderRadius: 6, padding: "0.2rem 0.5rem", cursor: "pointer", fontSize: 11 },
  piiBadge: { fontSize: 10, background: theme.amberBg, color: theme.amber, border: `1px solid ${theme.amber}`, padding: "0.15rem 0.5rem", borderRadius: 999, fontWeight: 700 },
  badge: { fontSize: 11, color: theme.bg, padding: "0.1rem 0.5rem", borderRadius: 999, fontWeight: 700, textTransform: "uppercase" },
  button: { padding: "0.3rem 0.7rem", borderRadius: 6, border: `1px solid ${theme.border}`, background: theme.bgHover, color: theme.text, cursor: "pointer", fontSize: 12 },
  disabled: { opacity: 0.5, cursor: "not-allowed" },
  input: { padding: "0.3rem 0.5rem", borderRadius: 6, border: `1px solid ${theme.border}`, background: theme.bgCard, color: theme.text, fontSize: 12 },
  toggle: { background: "none", border: "none", color: theme.textDim, padding: "0.2rem 0.6rem", borderRadius: 999, cursor: "pointer", fontSize: 12, fontWeight: 600 },
  toggleActive: { background: theme.accent, color: theme.onAccent },
};
