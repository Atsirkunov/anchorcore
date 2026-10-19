import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api } from "../api";
import { theme } from "../theme";
import { basename, docTypeLabel, kindLabel, needsLook, shortRef, statusLabel } from "../entityLabels";
import type { ConflictLink, Dispute, Entity, MergeProposal, Section, VocabProposal } from "../types";

type RowContext = {
  source_name: string | null;
  source_ref: string;
  window_text: string;
  expanded_before: string[];
  expanded_after: string[];
  highlight: string;
  item_title: string;
  full_text?: string;
};

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

function trustTitle(status: string): string {
  if (status === "unverified") return "Nobody has checked this yet";
  if (status === "verified") return "Included in answers";
  if (status === "disputed") return "Excluded from answers until re-checked";
  return status;
}

function statusStyle(status: string): React.CSSProperties {
  if (status === "unverified") return { background: theme.amberBg, color: theme.amber, border: `1px solid ${theme.amber}` };
  if (status === "verified") return { background: theme.greenBg, color: theme.greenText };
  if (status === "disputed") return { background: theme.redBg, color: theme.redText, border: `1px solid ${theme.redBorder}` };
  return { background: theme.mutedBg, color: theme.textMuted };
}

function docTypeStyle(docType: string): React.CSSProperties {
  const color = docType === "code" ? theme.blue : docType === "record" ? theme.accentAlt : theme.textDim;
  return {
    fontSize: 10,
    background: docType === "code" ? theme.blueBg : "transparent",
    color,
    border: `1px solid ${color}`,
    padding: "0.15rem 0.5rem",
    borderRadius: 999,
    fontWeight: 700,
  };
}

function ConflictBlock({
  entityId,
  lookup,
  ensureSummaries,
  idOptions,
}: {
  entityId: number;
  lookup: (id: number) => Entity | null;
  ensureSummaries: (ids: number[]) => void;
  idOptions: { id: number; summary: string }[];
}) {
  const [links, setLinks] = useState<ConflictLink[] | null>(null);
  const [open, setOpen] = useState(false);
  const [other, setOther] = useState("");
  const [note, setNote] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function load() {
    try {
      setError(null);
      const list = await api.listConflicts(entityId);
      setLinks(list);
      ensureSummaries(list.map((l) => l.other_entity_id));
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to load conflicts");
    }
  }

  async function toggle() {
    const next = !open;
    setOpen(next);
    if (next && links === null) await load();
  }

  async function flag() {
    const otherId = Number(other.trim().replace(/^#/, ""));
    if (!Number.isInteger(otherId) || otherId <= 0) {
      setError("Pick the contradicting item from the list (its #id).");
      return;
    }
    if (otherId === entityId) {
      setError("An item can't contradict itself — pick a different #id.");
      return;
    }
    setBusy(true);
    try {
      setError(null);
      await api.flagConflict(entityId, otherId, note.trim() || undefined);
      setOther("");
      setNote("");
      await load();
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to flag conflict");
    } finally {
      setBusy(false);
    }
  }

  async function unflag(otherId: number) {
    setBusy(true);
    try {
      setError(null);
      await api.unflagConflict(entityId, otherId);
      await load();
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to remove conflict");
    } finally {
      setBusy(false);
    }
  }

  const listId = `conflict-ids-${entityId}`;
  return (
    <div style={{ marginTop: 6 }}>
      <button onClick={toggle} style={styles.link}>
        {open ? "Hide conflicts ▲" : `Conflicts${links !== null && links.length > 0 ? ` (${links.length})` : ""} ▼`}
      </button>
      {open && (
        <div style={{ marginTop: 6, display: "grid", gap: 6 }}>
          {links === null && <div style={{ fontSize: 12, color: theme.textDim }}>Loading…</div>}
          {links !== null && links.length === 0 && <div style={{ fontSize: 12, color: theme.textDim }}>No conflicts flagged.</div>}
          {links !== null &&
            links.map((l) => {
              const target = lookup(l.other_entity_id);
              return (
                <div key={l.other_entity_id} style={{ fontSize: 12, display: "flex", gap: 8, alignItems: "baseline", flexWrap: "wrap" }}>
                  <span style={{ color: theme.amber, fontWeight: 700 }}>contradicts #{l.other_entity_id}</span>
                  {target ? (
                    <span style={{ color: theme.text }} title={target.summary}>
                      “{target.summary.length > 90 ? `${target.summary.slice(0, 90)}…` : target.summary}”
                    </span>
                  ) : (
                    <span style={{ color: theme.textDim }}>(item not loaded)</span>
                  )}
                  {l.note && <span style={{ color: theme.textMuted }}>{l.note}</span>}
                  <button onClick={() => unflag(l.other_entity_id)} disabled={busy} style={{ ...styles.button, ...(busy ? styles.disabled : {}) }}>
                    Remove
                  </button>
                </div>
              );
            })}
          <div style={{ display: "flex", gap: 6, flexWrap: "wrap", alignItems: "center" }}>
            <input
              value={other}
              onChange={(e) => setOther(e.target.value)}
              placeholder="Contradicting #id"
              list={listId}
              style={{ ...styles.input, width: 130 }}
            />
            <datalist id={listId}>
              {idOptions.map((o) => (
                <option key={o.id} value={`#${o.id}`}>
                  {o.summary}
                </option>
              ))}
            </datalist>
            <input value={note} onChange={(e) => setNote(e.target.value)} placeholder="Why do they contradict? (optional)" style={{ ...styles.input, flex: 1, minWidth: 180 }} />
            <button onClick={flag} disabled={busy} style={{ ...styles.button, ...(busy ? styles.disabled : {}) }}>
              {busy ? "…" : "Flag conflict"}
            </button>
          </div>
          {error && <div style={{ fontSize: 12, color: theme.red }}>{error}</div>}
        </div>
      )}
    </div>
  );
}

function DisputeBox({
  what,
  reason,
  onReason,
  onRecord,
  onCancel,
  busy,
}: {
  what: string;
  reason: string;
  onReason: (v: string) => void;
  onRecord: () => void;
  onCancel: () => void;
  busy: boolean;
}) {
  return (
    <div style={{ display: "flex", gap: 6, alignItems: "center", flexWrap: "wrap", marginTop: 6 }}>
      <input
        style={{ ...styles.input, flex: 1, minWidth: 220 }}
        autoFocus
        placeholder={`Why is ${what} wrong? (recorded — visible under History)`}
        value={reason}
        onChange={(ev) => onReason(ev.target.value)}
        onKeyDown={(ev) => ev.key === "Enter" && onRecord()}
      />
      <button
        style={{ ...styles.button, background: theme.redBg, color: theme.redText, ...(busy ? styles.disabled : {}) }}
        disabled={busy}
        onClick={onRecord}
        title="Records the dispute and removes it from answers until re-checked"
      >
        {busy ? "…" : "Record dispute"}
      </button>
      <button style={styles.button} onClick={onCancel}>
        Cancel
      </button>
    </div>
  );
}

function HistoryBlock({
  open,
  disputes,
  onToggle,
}: {
  open: boolean;
  disputes: Dispute[] | null;
  onToggle: () => void;
}) {
  return (
    <div style={{ marginTop: 6 }}>
      <button onClick={onToggle} style={styles.link}>
        {open ? "Hide history ▲" : `History${disputes !== null && disputes.length > 0 ? ` (${disputes.length})` : ""} ▼`}
      </button>
      {open && (
        <div style={{ marginTop: 6, display: "grid", gap: 4 }}>
          {disputes === null && <div style={{ fontSize: 12, color: theme.textDim }}>Loading…</div>}
          {disputes !== null && disputes.length === 0 && <div style={{ fontSize: 12, color: theme.textDim }}>No disputes recorded.</div>}
          {disputes !== null &&
            disputes.map((d) => (
              <div key={d.id} style={{ fontSize: 12, color: theme.textMuted }}>
                <span style={{ color: theme.redText }}>⚑ {d.reason || "(no reason given)"}</span>
                {d.user && <span> — {d.user}</span>}
                <span style={{ color: theme.textDim }}> · {d.created_at.slice(0, 10)}</span>
              </div>
            ))}
        </div>
      )}
    </div>
  );
}

const KIND_COLORS: Record<string, string> = {
  decision: theme.purple,
  document: theme.blue,
  action: theme.amber,
  note: theme.textDim,
};

type EntityRowProps = {
  e: Entity;
  showRef: boolean;
  sections: Section[] | undefined;
  kindOptions: string[];
  idOptions: { id: number; summary: string }[];
  contextOpen: boolean;
  context?: RowContext;
  busy: boolean;
  disputing: boolean;
  disputeReason: string;
  historyOpen: boolean;
  disputes: Dispute[] | null;
  lookup: (id: number) => Entity | null;
  ensureSummaries: (ids: number[]) => void;
  onToggleContext: () => void;
  onVerify: () => void;
  onStartDispute: () => void;
  onCancelDispute: () => void;
  onDisputeReason: (v: string) => void;
  onRecordDispute: () => void;
  onToggleHistory: () => void;
  onKindChange: (kind: string) => void;
};

function EntityRow(props: EntityRowProps) {
  const { e } = props;
  const bucket = docTypeLabel(e.doc_type);
  return (
    <article style={{ ...styles.card, borderLeft: `4px solid ${KIND_COLORS[e.kind] ?? theme.textDim}` }}>
      <div style={{ display: "flex", gap: 6, alignItems: "center", flexWrap: "wrap" }}>
        <span style={{ fontSize: 11, color: theme.textMuted }} title={`Item #${e.id}`}>
          #{e.id}
        </span>
        <span style={{ ...styles.badge, background: KIND_COLORS[e.kind] ?? theme.textDim }} title={`Type: ${e.kind}`}>
          {kindLabel(e.kind)}
        </span>
        {bucket && (
          <span style={docTypeStyle(e.doc_type as string)} title={e.doc_type === "code" ? "Derived from source code" : `From a ${e.doc_type} document`}>
            {bucket}
          </span>
        )}
        <span style={{ ...styles.statusBadge, ...statusStyle(e.status) }} title={trustTitle(e.status)}>
          {statusLabel(e.status)}
        </span>
        {e.dispute_count > 0 && (
          <span title={`Disputed ${e.dispute_count}× — excluded from Q&A`} style={{ fontSize: 11, color: theme.redText }}>
            ⚑ disputed ×{e.dispute_count}
          </span>
        )}
        <span style={{ fontSize: 11, color: theme.textMuted }} title="The classifier's own certainty — not a fact about the text">
          conf {(e.confidence * 100).toFixed(0)}%
        </span>
        {props.showRef && (
          <span
            style={{ marginLeft: "auto", fontSize: 11, color: theme.textDim, maxWidth: 220, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}
            title={e.source_ref}
          >
            {shortRef(e.source_ref)}
          </span>
        )}
      </div>
      {props.showRef && props.sections && props.sections.length > 0 && (
        <div style={{ marginTop: 4 }}>
          <Breadcrumb path={props.sections[0].path} />
        </div>
      )}
      <p style={{ margin: "0.4rem 0", fontWeight: 600 }}>{e.summary}</p>
      {e.reasoning && <p style={{ margin: 0, color: theme.textMuted, fontSize: 13 }}>{e.reasoning}</p>}
      {e.window_text && (
        <div style={{ marginTop: 6 }}>
          <button onClick={props.onToggleContext} style={styles.link}>
            {props.contextOpen ? "Hide context ▲" : "Show context ▼"}
          </button>
          {props.contextOpen && (
            <div style={{ marginTop: 6, padding: "0.6rem 0.75rem", background: theme.bgElevated, border: `1px solid ${theme.border}`, borderRadius: 6, maxHeight: 360, overflowY: "auto" }}>
              <div style={{ fontSize: 11, color: theme.textDim, textTransform: "uppercase", marginBottom: 4 }}>
                {props.context?.item_title || "Source"} · {shortRef(props.context?.source_ref || e.source_ref)}
              </div>
              {props.context?.expanded_before.map((t, i) => (
                <pre key={i} style={{ whiteSpace: "pre-wrap", margin: "6px 0", fontSize: 12, color: theme.textDim, opacity: 0.85 }}>
                  {t}
                </pre>
              ))}
              <pre style={{ whiteSpace: "pre-wrap", margin: "6px 0", fontSize: 12, background: theme.bg, border: `1px solid ${theme.border}`, borderRadius: 6, padding: "0.5rem 0.6rem", color: theme.text }}>
                {props.context?.window_text || e.window_text}
              </pre>
              {props.context?.expanded_after.map((t, i) => (
                <pre key={i} style={{ whiteSpace: "pre-wrap", margin: "6px 0", fontSize: 12, color: theme.textDim, opacity: 0.85 }}>
                  {t}
                </pre>
              ))}
            </div>
          )}
        </div>
      )}
      <ConflictBlock entityId={e.id} lookup={props.lookup} ensureSummaries={props.ensureSummaries} idOptions={props.idOptions} />
      <HistoryBlock open={props.historyOpen} disputes={props.disputes} onToggle={props.onToggleHistory} />
      <div style={{ display: "flex", gap: 8, marginTop: 8, flexWrap: "wrap", alignItems: "center" }}>
        <button
          style={{ ...styles.button, ...(props.busy ? styles.disabled : {}) }}
          disabled={props.busy}
          onClick={props.onVerify}
          title="Keeps this in answers"
        >
          {props.busy ? "…" : "Looks right"}
        </button>
        {!props.disputing ? (
          <button
            style={{ ...styles.button, ...(props.busy ? styles.disabled : {}) }}
            disabled={props.busy}
            onClick={props.onStartDispute}
            title="Removes this from answers until re-checked"
          >
            {props.busy ? "…" : "Dispute…"}
          </button>
        ) : null}
        <select
          value={e.kind}
          onChange={(ev) => props.onKindChange(ev.target.value)}
          style={styles.input}
          title="Change the type"
          disabled={props.busy}
        >
          {props.kindOptions.map((k) => (
            <option key={k} value={k}>
              {kindLabel(k)}
            </option>
          ))}
        </select>
      </div>
      {props.disputing && (
        <DisputeBox what="this item" reason={props.disputeReason} onReason={props.onDisputeReason} onRecord={props.onRecordDispute} onCancel={props.onCancelDispute} busy={props.busy} />
      )}
    </article>
  );
}

function DuplicatesSection({
  dupes,
  lookup,
  busyId,
  onDecide,
}: {
  dupes: MergeProposal[];
  lookup: (id: number) => Entity | null;
  busyId: number | null;
  onDecide: (proposalId: number, decision: "merge" | "dismiss") => void;
}) {
  if (dupes.length === 0) return null;
  return (
    <section style={{ ...styles.docCard, borderLeft: `4px solid ${theme.amber}`, marginBottom: 12 }}>
      <div style={{ fontWeight: 700, fontSize: 13 }}>Possible doubles ({dupes.length})</div>
      <div style={{ fontSize: 12, color: theme.textDim, margin: "2px 0 8px" }}>
        These pairs look like the same thing twice. Merging keeps A and removes B — it can&apos;t be undone.
      </div>
      <div style={{ display: "grid", gap: 8 }}>
        {dupes.map((d) => {
          const a = lookup(d.entity_a_id);
          const b = lookup(d.entity_b_id);
          return (
            <article key={d.id} style={styles.cardSmall}>
              <div style={{ display: "grid", gap: 4, fontSize: 12 }}>
                <div>
                  <span style={{ fontWeight: 700, color: theme.text }}>A · #{d.entity_a_id}</span>{" "}
                  {a ? (
                    <>
                      <span style={{ color: theme.textMuted }}>[{kindLabel(a.kind)}]</span> {a.summary}{" "}
                      <span style={{ color: theme.textDim }}>· {shortRef(a.source_ref)}</span>
                    </>
                  ) : (
                    <span style={{ color: theme.textDim }}>(loading…)</span>
                  )}
                </div>
                <div>
                  <span style={{ fontWeight: 700, color: theme.text }}>B · #{d.entity_b_id}</span>{" "}
                  {b ? (
                    <>
                      <span style={{ color: theme.textMuted }}>[{kindLabel(b.kind)}]</span> {b.summary}{" "}
                      <span style={{ color: theme.textDim }}>· {shortRef(b.source_ref)}</span>
                    </>
                  ) : (
                    <span style={{ color: theme.textDim }}>(loading…)</span>
                  )}
                </div>
              </div>
              <div style={{ display: "flex", gap: 8, marginTop: 8 }}>
                <button
                  style={{ ...styles.button, background: theme.greenBg, ...(busyId !== null ? styles.disabled : {}) }}
                  disabled={busyId !== null}
                  onClick={() => onDecide(d.id, "merge")}
                  title={`Merge into #${d.entity_a_id} and remove #${d.entity_b_id}`}
                >
                  {busyId === d.id ? "Merging…" : `Merge — keeps #${d.entity_a_id}`}
                </button>
                <button style={{ ...styles.button, ...(busyId !== null ? styles.disabled : {}) }} disabled={busyId !== null} onClick={() => onDecide(d.id, "dismiss")}>
                  {busyId === d.id ? "…" : "Not doubles"}
                </button>
              </div>
            </article>
          );
        })}
      </div>
    </section>
  );
}

function VocabularySection({
  proposals,
  busyId,
  onDecide,
}: {
  proposals: VocabProposal[];
  busyId: number | null;
  onDecide: (id: number, decision: "approve" | "reject") => void;
}) {
  if (proposals.length === 0) return null;
  return (
    <section style={{ ...styles.docCard, borderLeft: `4px solid ${theme.amber}`, marginBottom: 12 }}>
      <div style={{ fontWeight: 700, fontSize: 13 }}>Words to confirm ({proposals.length})</div>
      <div style={{ fontSize: 12, color: theme.textDim, margin: "2px 0 8px" }}>
        The app noticed these terms while reading your documents. Confirming teaches search that the
        short form means the long one — rejecting hides the pair for good.
      </div>
      <div style={{ display: "grid", gap: 8 }}>
        {proposals.map((p) => (
          <article key={p.id} style={styles.cardSmall}>
            <div style={{ fontSize: 12 }}>
              <span style={{ fontWeight: 700, color: theme.text }}>
                “{p.term}” means “{p.expansion}”
              </span>{" "}
              <span style={{ color: theme.textDim }}>
                · seen in {p.item_title || p.external_id}
                {p.confirmations > 1 ? ` (+${p.confirmations - 1} more)` : ""}
              </span>
            </div>
            <div style={{ display: "flex", gap: 8, marginTop: 8 }}>
              <button
                style={{ ...styles.button, background: theme.greenBg, ...(busyId !== null ? styles.disabled : {}) }}
                disabled={busyId !== null}
                onClick={() => onDecide(p.id, "approve")}
                title={`Teach search that "${p.term}" means "${p.expansion}"`}
              >
                {busyId === p.id ? "Confirming…" : "Confirm"}
              </button>
              <button
                style={{ ...styles.button, ...(busyId !== null ? styles.disabled : {}) }}
                disabled={busyId !== null}
                onClick={() => onDecide(p.id, "reject")}
                title="Hide this pair — it will never be suggested again"
              >
                {busyId === p.id ? "…" : "Reject"}
              </button>
            </div>
          </article>
        ))}
      </div>
    </section>
  );
}

const KNOWN_STATUSES = ["unverified", "verified", "disputed"];
const KIND_ORDER = ["decision", "action", "note", "document"];

type DisputeTarget = { scope: "single"; id: number } | { scope: "bulk"; itemId: number; ids: number[] } | null;

export function EntitiesTab() {
  const [entities, setEntities] = useState<Entity[]>([]);
  const [kind, setKind] = useState<string>("");
  const [statusFilter, setStatusFilter] = useState<string>("");
  const [error, setError] = useState<string | null>(null);
  const [busyId, setBusyId] = useState<number | null>(null);
  const [bulkBusy, setBulkBusy] = useState<number | null>(null);
  const [expanded, setExpanded] = useState<number | null>(null);
  const [ctx, setCtx] = useState<Record<number, RowContext>>({});
  const [groupedPref, setGroupedPref] = useState<boolean>(() => {
    try {
      return localStorage.getItem("entities-grouped") !== "0";
    } catch {
      return true;
    }
  });
  const [expandedDocs, setExpandedDocs] = useState<Set<number>>(new Set());
  const [docCtx, setDocCtx] = useState<Record<number, { item_title: string; source_ref: string; full_text: string; source_name: string | null }>>({});
  const [sectionsByItem, setSectionsByItem] = useState<Record<number, Section[]>>({});
  const [dupes, setDupes] = useState<MergeProposal[]>([]);
  const [busyProposal, setBusyProposal] = useState<number | null>(null);
  const [vocab, setVocab] = useState<VocabProposal[]>([]);
  const [busyVocab, setBusyVocab] = useState<number | null>(null);
  const [extras, setExtras] = useState<Record<number, Entity>>({});
  const [disputeTarget, setDisputeTarget] = useState<DisputeTarget>(null);
  const [disputeReason, setDisputeReason] = useState("");
  const [historyOpen, setHistoryOpen] = useState<number | null>(null);
  const [disputes, setDisputes] = useState<Record<number, Dispute[]>>({});
  const fetchedIds = useRef<Set<number>>(new Set());

  function setGrouped(v: boolean) {
    setGroupedPref(v);
    try {
      localStorage.setItem("entities-grouped", v ? "1" : "0");
    } catch {
      // private mode — pref just doesn't persist
    }
  }

  const refresh = useCallback(() => {
    const statusParam = statusFilter && statusFilter !== "needs_review" ? statusFilter : undefined;
    api
      .listEntities({ kind: kind || undefined, status: statusParam, limit: 2000 })
      .then(setEntities)
      .catch((e) => setError(String(e)));
  }, [kind, statusFilter]);

  const refreshDupes = useCallback(() => {
    api
      .duplicates()
      .then(setDupes)
      .catch((e) => setError(String(e)));
  }, []);

  const refreshVocab = useCallback(() => {
    api
      .vocabulary()
      .then(setVocab)
      .catch((e) => setError(String(e)));
  }, []);

  useEffect(() => refresh(), [refresh]);
  useEffect(() => refreshDupes(), [refreshDupes]);
  useEffect(() => refreshVocab(), [refreshVocab]);

  const byId = useMemo(() => {
    const m = new Map<number, Entity>();
    for (const e of entities) m.set(e.id, e);
    for (const e of Object.values(extras)) m.set(e.id, e);
    return m;
  }, [entities, extras]);

  const lookup = useCallback((id: number): Entity | null => byId.get(id) ?? null, [byId]);

  const ensureSummaries = useCallback(
    (ids: number[]) => {
      const missing = ids.filter((id) => !byId.has(id) && !fetchedIds.current.has(id));
      if (missing.length === 0) return;
      for (const id of missing) fetchedIds.current.add(id);
      void Promise.allSettled(missing.map((id) => api.getEntity(id))).then((results) => {
        const found: Record<number, Entity> = {};
        results.forEach((r, i) => {
          if (r.status === "fulfilled") found[missing[i]] = r.value;
        });
        if (Object.keys(found).length > 0) setExtras((m) => ({ ...m, ...found }));
      });
    },
    [byId],
  );

  useEffect(() => {
    ensureSummaries(dupes.flatMap((d) => [d.entity_a_id, d.entity_b_id]));
  }, [dupes, ensureSummaries]);

  async function toggleCtx(e: Entity) {
    const next = expanded === e.id ? null : e.id;
    setExpanded(next);
    if (next !== null && !ctx[e.id] && e.window_text) {
      try {
        const c = await api.entityContext(e.id);
        setCtx((m) => ({ ...m, [e.id]: c }));
      } catch {
        void 0;
      }
    }
  }

  async function toggleHistory(id: number) {
    const next = historyOpen === id ? null : id;
    setHistoryOpen(next);
    if (next !== null && disputes[id] == null) {
      try {
        const d = await api.entityDisputes(id);
        setDisputes((m) => ({ ...m, [id]: d }));
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
        setDisputes((m) => ({ ...m, [id]: [] }));
      }
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

  async function recordDispute() {
    if (!disputeTarget) return;
    const reason = disputeReason;
    if (disputeTarget.scope === "single") {
      const id = disputeTarget.id;
      setBusyId(id);
      setError(null);
      try {
        await api.disputeEntity(id, reason);
        setDisputeTarget(null);
        setDisputeReason("");
        refresh();
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
      } finally {
        setBusyId(null);
      }
      return;
    }
    const itemId = disputeTarget.itemId;
    const ids = disputeTarget.ids;
    setBulkBusy(itemId);
    setError(null);
    try {
      for (const id of ids) {
        await api.disputeEntity(id, reason);
      }
      setDisputeTarget(null);
      setDisputeReason("");
      refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      refresh();
    } finally {
      setBulkBusy(null);
    }
  }

  async function bulkVerify(itemId: number, list: Entity[]) {
    setBulkBusy(itemId);
    setError(null);
    try {
      for (const e of list) {
        await api.updateEntity(e.id, { status: "verified" });
      }
      refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      refresh();
    } finally {
      setBulkBusy(null);
    }
  }

  async function decide(proposalId: number, decision: "merge" | "dismiss") {
    setBusyProposal(proposalId);
    setError(null);
    try {
      await api.decideMerge(proposalId, decision);
      fetchedIds.current.clear();
      setExtras({});
      refresh();
      await refreshDupes();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusyProposal(null);
    }
  }

  async function decideVocab(id: number, decision: "approve" | "reject") {
    setBusyVocab(id);
    setError(null);
    try {
      await api.decideVocab(id, decision);
      await refreshVocab();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusyVocab(null);
    }
  }

  const needCount = useMemo(() => entities.filter((e) => needsLook(e.status)).length, [entities]);

  const kindOptions = useMemo(() => {
    const distinct = Array.from(new Set(entities.map((e) => e.kind)));
    return distinct.sort((a, b) => {
      const ai = KIND_ORDER.indexOf(a);
      const bi = KIND_ORDER.indexOf(b);
      if (ai === -1 && bi === -1) return a.localeCompare(b);
      if (ai === -1) return 1;
      if (bi === -1) return -1;
      return ai - bi;
    });
  }, [entities]);

  const extraStatuses = useMemo(
    () => Array.from(new Set(entities.map((e) => e.status))).filter((s) => !KNOWN_STATUSES.includes(s)).sort(),
    [entities],
  );

  const idOptions = useMemo(() => entities.map((e) => ({ id: e.id, summary: e.summary })), [entities]);

  const filtered = useMemo(() => {
    return entities.filter((e) => {
      if (kind && e.kind !== kind) return false;
      if (statusFilter === "needs_review") return needsLook(e.status);
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
    // priority: docs with most items needing a look first, then largest, then id
    return Array.from(m.entries()).sort((a, b) => {
      const aNeed = a[1].filter((e) => needsLook(e.status)).length;
      const bNeed = b[1].filter((e) => needsLook(e.status)).length;
      if (bNeed !== aNeed) return bNeed - aNeed;
      return b[1].length - a[1].length || a[0] - b[0];
    });
  }, [filtered]);

  // prefetch sections for doc headers + TOCs (first 30 docs)
  useEffect(() => {
    if (groups.length === 0) return;
    const secFetch = groups.slice(0, 30).filter(([id]) => sectionsByItem[id] == null);
    for (const [itemId] of secFetch) {
      api
        .sections({ item_id: itemId })
        .then((secs) => setSectionsByItem((m) => ({ ...m, [itemId]: secs })))
        .catch(() => {});
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [groups]);

  // flat view: also fetch sections for first 30 entities by item_id
  useEffect(() => {
    if (groupedPref) return;
    const ids = Array.from(new Set(filtered.slice(0, 30).map((e) => e.item_id).filter((v): v is number => v != null)));
    for (const itemId of ids) {
      if (sectionsByItem[itemId] == null) {
        api
          .sections({ item_id: itemId })
          .then((secs) => setSectionsByItem((m) => ({ ...m, [itemId]: secs })))
          .catch(() => {});
      }
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [filtered, groupedPref]);

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
          setDocCtx((m) => ({ ...m, [itemId]: { item_title: c.item_title, source_ref: c.source_ref, full_text: c.full_text ?? c.window_text, source_name: c.source_name } }));
        } catch {
          void 0;
        }
      }
      if (sectionsByItem[itemId] == null) {
        try {
          const secs = await api.sections({ item_id: itemId });
          setSectionsByItem((m) => ({ ...m, [itemId]: secs }));
        } catch {
          void 0;
        }
      }
    }
  }

  function clearFilters() {
    setKind("");
    setStatusFilter("");
  }

  function rowProps(e: Entity, showRef: boolean): EntityRowProps {
    return {
      e,
      showRef,
      sections: e.item_id != null ? sectionsByItem[e.item_id] : undefined,
      kindOptions,
      idOptions,
      contextOpen: expanded === e.id,
      context: ctx[e.id],
      busy: busyId === e.id,
      disputing: disputeTarget?.scope === "single" && disputeTarget.id === e.id,
      disputeReason,
      historyOpen: historyOpen === e.id,
      disputes: disputes[e.id] ?? null,
      lookup,
      ensureSummaries,
      onToggleContext: () => toggleCtx(e),
      onVerify: () => update(e, { status: "verified" }),
      onStartDispute: () => {
        setDisputeTarget({ scope: "single", id: e.id });
        setDisputeReason("");
      },
      onCancelDispute: () => {
        setDisputeTarget(null);
        setDisputeReason("");
      },
      onDisputeReason: setDisputeReason,
      onRecordDispute: recordDispute,
      onToggleHistory: () => toggleHistory(e.id),
      onKindChange: (k) => update(e, { kind: k }),
    };
  }

  const filtersActive = kind !== "" || statusFilter !== "";

  return (
    <div>
      <div style={{ display: "flex", alignItems: "center", gap: 12, marginBottom: 12, flexWrap: "wrap" }}>
        <h2 style={{ margin: 0 }}>Entities</h2>
        <select value={kind} onChange={(e) => setKind(e.target.value)} style={styles.input} title="Filter by type">
          <option value="">All types</option>
          {kindOptions.map((k) => (
            <option key={k} value={k}>
              {kindLabel(k)}s
            </option>
          ))}
        </select>
        <select value={statusFilter} onChange={(e) => setStatusFilter(e.target.value)} style={styles.input} title="Filter by trust state">
          <option value="">All items</option>
          <option value="needs_review">Needs a look ({needCount})</option>
          <option value="unverified">New</option>
          <option value="verified">In answers</option>
          <option value="disputed">Disputed</option>
          {extraStatuses.map((s) => (
            <option key={s} value={s}>
              {statusLabel(s)}
            </option>
          ))}
        </select>
        <span style={{ fontSize: 12, color: statusFilter === "needs_review" ? theme.amber : theme.textDim }}>
          {filtered.length} / {entities.length} items{groupedPref ? ` · ${groups.length} documents` : ""} · {needCount} need a look
        </span>
        <div style={{ marginLeft: "auto", display: "flex", gap: 4, alignItems: "center", border: `1px solid ${theme.border}`, borderRadius: 999, padding: 2, background: theme.bgCard }}>
          <button onClick={() => setGrouped(true)} style={{ ...styles.toggle, ...(groupedPref ? styles.toggleActive : {}) }}>
            Grouped
          </button>
          <button onClick={() => setGrouped(false)} style={{ ...styles.toggle, ...(!groupedPref ? styles.toggleActive : {}) }}>
            Flat
          </button>
        </div>
      </div>
      {error && <p style={{ color: theme.red }}>{error}</p>}
      <DuplicatesSection dupes={dupes} lookup={lookup} busyId={busyProposal} onDecide={decide} />
      <VocabularySection proposals={vocab} busyId={busyVocab} onDecide={decideVocab} />
      {entities.length === 0 ? (
        <p style={{ color: theme.textDim }}>No entities yet. Ingest a source first.</p>
      ) : filtered.length === 0 ? (
        <div>
          <p style={{ color: theme.textDim }}>
            Nothing here — {entities.length} items total{statusFilter === "needs_review" ? ", none need a look." : "."}
          </p>
          {filtersActive && (
            <button onClick={clearFilters} style={styles.button}>
              Show all items
            </button>
          )}
        </div>
      ) : groupedPref ? (
        <div style={{ display: "grid", gap: 10 }}>
          {groups.map(([itemId, list]) => {
            const first = list[0];
            const isOpen = expandedDocs.has(itemId);
            const docTitle = docCtx[itemId]?.item_title || basename(first.source_ref) || `Doc #${itemId}`;
            const docRef = docCtx[itemId]?.source_ref || first.source_ref;
            const need = list.filter((e) => needsLook(e.status)).length;
            const bulkDisputing = disputeTarget?.scope === "bulk" && disputeTarget.itemId === itemId;
            const bulkRunning = bulkBusy === itemId;
            return (
              <section key={itemId} style={{ ...styles.docCard, borderLeft: `4px solid ${need > 0 ? theme.accent : theme.greenBg}` }}>
                <div style={{ display: "flex", gap: 10, alignItems: "center", flexWrap: "wrap" }}>
                  <div style={{ flex: 1, minWidth: 220 }}>
                    <div style={{ fontWeight: 700, fontSize: 13, color: theme.text, whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>{docTitle}</div>
                    <div style={{ fontSize: 11, color: theme.textDim, whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }} title={docRef}>
                      {shortRef(docRef)}
                    </div>
                    {sectionsByItem[itemId] && sectionsByItem[itemId].length > 0 && (
                      <div style={{ marginTop: 4 }}>
                        <Breadcrumb path={sectionsByItem[itemId][0].path} />
                      </div>
                    )}
                  </div>
                  <span style={styles.docCount}>{list.length} items</span>
                  {need > 0 && <span style={styles.needBadge}>{need} need a look</span>}
                  <button onClick={() => toggleDoc(itemId, first.id)} style={styles.docToggle}>
                    {isOpen ? "Hide document ▲" : "Show document ▼"}
                  </button>
                </div>
                {isOpen && docCtx[itemId]?.full_text && (
                  <div style={{ marginTop: 8, padding: "0.6rem 0.75rem", background: theme.bgElevated, border: `1px solid ${theme.border}`, borderRadius: 6, maxHeight: 320, overflowY: "auto" }}>
                    <div style={{ fontSize: 11, color: theme.textDim, textTransform: "uppercase", marginBottom: 4 }}>Whole doc — {docTitle}</div>
                    {sectionsByItem[itemId] && sectionsByItem[itemId].length > 1 && (
                      <div style={{ marginBottom: 8, display: "grid", gap: 3, padding: "0.4rem 0.5rem", background: theme.bg, border: `1px solid ${theme.border}`, borderRadius: 6 }}>
                        <div style={{ fontSize: 10, color: theme.textDim, textTransform: "uppercase" }}>Sections</div>
                        {sectionsByItem[itemId].map((s) => (
                          <div key={s.id} style={{ fontSize: 11 }}>
                            <Breadcrumb path={s.path || s.title} />
                          </div>
                        ))}
                      </div>
                    )}
                    <pre style={{ whiteSpace: "pre-wrap", margin: 0, fontSize: 12, color: theme.textMuted, lineHeight: 1.5 }}>{docCtx[itemId].full_text.slice(0, 16000)}</pre>
                  </div>
                )}
                <div style={{ display: "flex", gap: 6, marginTop: 8, flexWrap: "wrap" }}>
                  <button
                    onClick={() => bulkVerify(itemId, list)}
                    disabled={bulkRunning}
                    style={{ ...styles.button, background: theme.accent, color: theme.onAccent, ...(bulkRunning ? styles.disabled : {}) }}
                    title="Marks every item below as looked-at; they stay in answers"
                  >
                    {bulkRunning ? "…" : `Looks right — all ${list.length} below`}
                  </button>
                  {!bulkDisputing ? (
                    <button
                      onClick={() => {
                        setDisputeTarget({ scope: "bulk", itemId, ids: list.map((e) => e.id) });
                        setDisputeReason("");
                      }}
                      disabled={bulkRunning}
                      style={{ ...styles.button, ...(bulkRunning ? styles.disabled : {}) }}
                      title="Records a dispute per item below and removes them from answers"
                    >
                      {bulkRunning ? "…" : `Dispute all ${list.length}…`}
                    </button>
                  ) : null}
                </div>
                {bulkDisputing && (
                  <DisputeBox
                    what={`these ${list.length} items`}
                    reason={disputeReason}
                    onReason={setDisputeReason}
                    onRecord={recordDispute}
                    onCancel={() => {
                      setDisputeTarget(null);
                      setDisputeReason("");
                    }}
                    busy={bulkRunning}
                  />
                )}
                <div style={{ display: "grid", gap: 6, marginTop: 10 }}>
                  {list.map((e) => (
                    <EntityRow key={e.id} {...rowProps(e, false)} />
                  ))}
                </div>
              </section>
            );
          })}
        </div>
      ) : (
        <div style={{ display: "grid", gap: 8 }}>
          {filtered.map((e) => (
            <EntityRow key={e.id} {...rowProps(e, true)} />
          ))}
        </div>
      )}
    </div>
  );
}

const styles: Record<string, React.CSSProperties> = {
  card: { background: theme.bgElevated, border: `1px solid ${theme.border}`, borderRadius: 7, padding: "0.6rem 0.8rem" },
  cardSmall: { background: theme.bgElevated, border: `1px solid ${theme.border}`, borderRadius: 7, padding: "0.6rem 0.8rem" },
  docCard: { background: theme.bgCard, border: `1px solid ${theme.border}`, borderRadius: 8, padding: "0.7rem 0.9rem" },
  docCount: { fontSize: 11, background: theme.bgHover, color: theme.textMuted, padding: "0.15rem 0.5rem", borderRadius: 999, fontWeight: 600 },
  needBadge: { fontSize: 10, background: theme.amberBg, color: theme.amber, border: `1px solid ${theme.amber}`, padding: "0.15rem 0.5rem", borderRadius: 999, fontWeight: 700 },
  docToggle: { background: "none", border: `1px solid ${theme.border}`, color: theme.accentAlt, borderRadius: 6, padding: "0.2rem 0.5rem", cursor: "pointer", fontSize: 11 },
  badge: { fontSize: 11, color: theme.bg, padding: "0.1rem 0.5rem", borderRadius: 999, fontWeight: 700, textTransform: "uppercase" },
  statusBadge: { fontSize: 10, padding: "0.15rem 0.5rem", borderRadius: 999, fontWeight: 700 },
  button: { padding: "0.3rem 0.7rem", borderRadius: 6, border: `1px solid ${theme.border}`, background: theme.bgHover, color: theme.text, cursor: "pointer", fontSize: 12 },
  disabled: { opacity: 0.5, cursor: "not-allowed" },
  input: { padding: "0.3rem 0.5rem", borderRadius: 6, border: `1px solid ${theme.border}`, background: theme.bgCard, color: theme.text, fontSize: 12 },
  toggle: { background: "none", border: "none", color: theme.textDim, padding: "0.2rem 0.6rem", borderRadius: 999, cursor: "pointer", fontSize: 12, fontWeight: 600 },
  toggleActive: { background: theme.accent, color: theme.onAccent },
  link: { background: "none", border: "none", color: theme.accentAlt, cursor: "pointer", fontSize: 12, padding: 0, textDecoration: "underline" },
};
