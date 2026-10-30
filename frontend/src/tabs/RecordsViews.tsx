// R45.9: Decisions + Actions views over `GET /decisions` / `GET /actions`
// (the same endpoints the MCP `decisions`/`actions` tools wrap). R45.25:
// a person can dispute a wrong record (hidden from these lists and the agent
// tools until restored) and find disputed rows under the "Disputed" filter.
import { useCallback, useEffect, useState } from "react";
import { api } from "../api";
import { theme, radius } from "../theme";
import type { ActionFilters, ActionRecord, DecisionRecord, RecordProvenance } from "../types";

function Provenance({ p }: { p: RecordProvenance }) {
  const where = p.path || p.title || `item #${p.item_id}`;
  return (
    <span style={styles.provenance} title={`item #${p.item_id}${p.chunk_id ? ` · chunk #${p.chunk_id}` : ""}`}>
      {where}
    </span>
  );
}

function GateNotice({ blocked, refusal }: { blocked: number; refusal: string | null }) {
  if (blocked === 0) return null;
  const text = refusal
    ? "Everything here comes from sensitive sources, and the answer provider isn't trusted for them."
    : `${blocked} row(s) from sensitive sources are hidden because the answer provider isn't trusted for them.`;
  return <p style={styles.notice}>{text}</p>;
}

const DISPUTED = "disputed";

/** R45.25: dispute with a required reason (`POST /entities/:id/dispute`,
 *  history kept) or restore (`PATCH status=unverified`). The row stays on
 *  screen with its new state until the list reloads, so a slip can be undone. */
function DisputeControl({ entityId, reviewStatus, onStatus }: { entityId: number; reviewStatus: string; onStatus: (id: number, status: string) => void }) {
  const [editing, setEditing] = useState(false);
  const [reason, setReason] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function run(work: () => Promise<{ status: string }>) {
    setBusy(true);
    setError(null);
    try {
      const updated = await work();
      onStatus(entityId, updated.status);
      setEditing(false);
      setReason("");
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  if (reviewStatus === DISPUTED) {
    return (
      <div style={styles.disputeRow}>
        <span style={styles.disputedNote}>Disputed — hidden from this list and from AI agents until restored.</span>
        <button style={styles.button} disabled={busy} onClick={() => run(() => api.updateEntity(entityId, { status: "unverified" }))}>
          {busy ? "…" : "Restore"}
        </button>
        {error && <span style={{ ...styles.meta, color: theme.red }}>{error}</span>}
      </div>
    );
  }
  if (!editing) {
    return (
      <div style={styles.disputeRow}>
        <button style={styles.link} onClick={() => setEditing(true)} title="Hides this record from the list and from AI agents until restored">
          Dispute…
        </button>
      </div>
    );
  }
  const record = () => reason.trim() && run(() => api.disputeEntity(entityId, reason.trim()));
  return (
    <div style={styles.disputeRow}>
      <input
        style={{ ...styles.input, flex: 1, minWidth: 220 }}
        autoFocus
        placeholder="Why is this wrong? (kept in the dispute history)"
        value={reason}
        onChange={(ev) => setReason(ev.target.value)}
        onKeyDown={(ev) => ev.key === "Enter" && record()}
      />
      <button style={{ ...styles.button, background: theme.redBg, color: theme.redText }} disabled={busy || !reason.trim()} onClick={record}>
        {busy ? "…" : "Record dispute"}
      </button>
      <button style={styles.button} onClick={() => setEditing(false)}>
        Cancel
      </button>
      {error && <span style={{ ...styles.meta, color: theme.red }}>{error}</span>}
    </div>
  );
}

/** Replace one row's review status in place (after a dispute or restore). */
function withStatus<T extends { entity_id: number; review_status: string }>(rows: T[], id: number, status: string): T[] {
  return rows.map((r) => (r.entity_id === id ? { ...r, review_status: status } : r));
}

function statusBadgeStyle(d: DecisionRecord): React.CSSProperties {
  if (!d.current) return { ...styles.badge, background: theme.mutedBg, color: theme.textMuted };
  if (d.status === "accepted") return { ...styles.badge, background: theme.greenBg, color: theme.greenText };
  return { ...styles.badge, background: theme.amberBg, color: theme.amber };
}

function DecisionRow({ d, onStatus }: { d: DecisionRecord; onStatus: (id: number, status: string) => void }) {
  return (
    <div style={{ ...styles.card, opacity: d.current ? 1 : 0.7 }}>
      <div style={styles.rowHead}>
        {d.record_id && <span style={styles.recordId}>{d.record_id}</span>}
        <span style={statusBadgeStyle(d)}>{d.status || "unknown"}</span>
        {d.owner && <span style={styles.meta}>Owner: {d.owner}</span>}
        <Provenance p={d.provenance} />
      </div>
      <div style={{ ...styles.summary, textDecoration: d.current ? "none" : "line-through" }}>{d.summary}</div>
      {(d.superseded_by || d.supersedes.length > 0) && (
        <div style={styles.meta}>
          {d.superseded_by && <span>Superseded by {d.superseded_by}. </span>}
          {d.supersedes.length > 0 && <span>Supersedes {d.supersedes.join(", ")}.</span>}
        </div>
      )}
      <DisputeControl entityId={d.entity_id} reviewStatus={d.review_status} onStatus={onStatus} />
    </div>
  );
}

export function DecisionsView() {
  const [rows, setRows] = useState<DecisionRecord[]>([]);
  const [gate, setGate] = useState({ blocked: 0, refusal: null as string | null });
  const [status, setStatus] = useState("current");
  const [q, setQ] = useState("");
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(() => {
    const filters = status === DISPUTED ? { disputed: true } : { status: status === "all" ? undefined : status };
    api
      .listDecisions({ ...filters, q: q.trim() || undefined })
      .then((res) => {
        setRows(res.decisions);
        setGate({ blocked: res.blocked, refusal: res.refusal });
        setError(null);
      })
      .catch((e: unknown) => {
        setError(e instanceof Error ? e.message : "Failed to load decisions");
      });
  }, [status, q]);

  useEffect(() => {
    load();
  }, [load]);

  return (
    <div>
      <div style={styles.toolbar}>
        <select value={status} onChange={(e) => setStatus(e.target.value)} style={styles.input} title="Filter by decision state">
          <option value="current">In force</option>
          <option value="all">All decisions</option>
          <option value="superseded">Superseded</option>
          <option value="deferred">Deferred</option>
          <option value="rejected">Rejected</option>
          <option value="proposed">Proposed</option>
          <option value={DISPUTED}>Disputed</option>
        </select>
        <input value={q} onChange={(e) => setQ(e.target.value)} placeholder="Search decisions" style={styles.input} />
        <span style={styles.meta}>{rows.length} decisions</span>
      </div>
      {error && <p style={{ color: theme.red }}>{error}</p>}
      <GateNotice {...gate} />
      {rows.length === 0 && !error ? (
        <p style={{ color: theme.textDim }}>
          No decisions found. Decisions come from records with a "Decision:" heading or field, or a "Status:" line.
        </p>
      ) : (
        <div style={styles.list}>
          {rows.map((d) => (
            <DecisionRow key={d.entity_id} d={d} onStatus={(id, s) => setRows((rs) => withStatus(rs, id, s))} />
          ))}
        </div>
      )}
    </div>
  );
}

function isOverdue(a: ActionRecord, today: string): boolean {
  return !a.done && /^\d{4}-\d{2}-\d{2}/.test(a.due) && a.due.slice(0, 10) < today;
}

function ActionRow({ a, today, onStatus }: { a: ActionRecord; today: string; onStatus: (id: number, status: string) => void }) {
  const overdue = isOverdue(a, today);
  return (
    <div style={styles.card}>
      <div style={styles.rowHead}>
        <span style={{ ...styles.badge, background: a.done ? theme.greenBg : theme.bgHover, color: a.done ? theme.greenText : theme.text }}>
          {a.done ? "done" : "open"}
        </span>
        <span style={styles.meta}>{a.owner || "No owner"}</span>
        {a.due && <span style={{ ...styles.meta, color: overdue ? theme.red : theme.textDim }}>Due {a.due}{overdue ? " (overdue)" : ""}</span>}
        <Provenance p={a.provenance} />
      </div>
      <div style={{ ...styles.summary, textDecoration: a.done ? "line-through" : "none" }}>{a.task}</div>
      <DisputeControl entityId={a.entity_id} reviewStatus={a.review_status} onStatus={onStatus} />
    </div>
  );
}

export function ActionsView() {
  const [rows, setRows] = useState<ActionRecord[]>([]);
  const [gate, setGate] = useState({ blocked: 0, refusal: null as string | null });
  const [owner, setOwner] = useState("");
  const [state, setState] = useState<NonNullable<ActionFilters["state"]> | typeof DISPUTED>("open");
  const [overdue, setOverdue] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const today = new Date().toISOString().slice(0, 10);

  const load = useCallback(() => {
    const filters = state === DISPUTED ? { state: "all" as const, disputed: true } : { state };
    api
      .listActions({ owner: owner.trim() || undefined, ...filters, overdue })
      .then((res) => {
        setRows(res.actions);
        setGate({ blocked: res.blocked, refusal: res.refusal });
        setError(null);
      })
      .catch((e: unknown) => {
        setError(e instanceof Error ? e.message : "Failed to load actions");
      });
  }, [owner, state, overdue]);

  useEffect(() => {
    load();
  }, [load]);

  return (
    <div>
      <div style={styles.toolbar}>
        <select value={state} onChange={(e) => setState(e.target.value as typeof state)} style={styles.input} title="Filter by state">
          <option value="open">Open</option>
          <option value="done">Done</option>
          <option value="all">All</option>
          <option value={DISPUTED}>Disputed</option>
        </select>
        <input value={owner} onChange={(e) => setOwner(e.target.value)} placeholder="Owner" style={styles.input} />
        <label style={styles.meta}>
          <input type="checkbox" checked={overdue} onChange={(e) => setOverdue(e.target.checked)} /> Overdue only
        </label>
        <span style={styles.meta}>{rows.length} actions</span>
      </div>
      {error && <p style={{ color: theme.red }}>{error}</p>}
      <GateNotice {...gate} />
      {rows.length === 0 && !error ? (
        <p style={{ color: theme.textDim }}>No actions found. Actions come from "Action items" sections and checkbox lists in records.</p>
      ) : (
        <div style={styles.list}>
          {rows.map((a) => (
            <ActionRow key={a.entity_id} a={a} today={today} onStatus={(id, s) => setRows((rs) => withStatus(rs, id, s))} />
          ))}
        </div>
      )}
    </div>
  );
}

const styles: Record<string, React.CSSProperties> = {
  toolbar: { display: "flex", alignItems: "center", gap: 12, marginBottom: 12, flexWrap: "wrap" },
  list: { display: "flex", flexDirection: "column", gap: 8 },
  card: { background: theme.bgElevated, border: `1px solid ${theme.border}`, borderRadius: radius, padding: "0.6rem 0.8rem" },
  rowHead: { display: "flex", alignItems: "center", gap: 8, flexWrap: "wrap", marginBottom: 4 },
  recordId: { fontSize: 11, fontFamily: theme.mono, fontWeight: 600, color: theme.accentAlt },
  badge: { fontSize: 10, fontFamily: theme.mono, padding: "0.15rem 0.5rem", borderRadius: radius, fontWeight: 600, textTransform: "uppercase", letterSpacing: "0.04em" },
  meta: { fontSize: 12, color: theme.textDim },
  provenance: { fontSize: 11, fontFamily: theme.mono, color: theme.textMuted, marginLeft: "auto" },
  summary: { fontSize: 15, fontFamily: theme.serif, color: theme.text },
  notice: { fontSize: 12, color: theme.amber, background: theme.amberBg, padding: "0.4rem 0.6rem", borderRadius: radius },
  disputeRow: { display: "flex", gap: 6, alignItems: "center", flexWrap: "wrap", marginTop: 6 },
  disputedNote: { fontSize: 12, color: theme.redText, background: theme.redBg, padding: "0.15rem 0.5rem", borderRadius: radius },
  button: { padding: "0.3rem 0.7rem", borderRadius: radius, border: `1px solid ${theme.border}`, background: theme.bgHover, color: theme.text, cursor: "pointer", fontSize: 12 },
  link: { background: "none", border: "none", color: theme.textDim, cursor: "pointer", fontSize: 12, padding: 0, textDecoration: "underline" },
  input: { padding: "0.3rem 0.5rem", borderRadius: radius, border: `1px solid ${theme.border}`, background: theme.bgCard, color: theme.text, fontSize: 12 },
};
