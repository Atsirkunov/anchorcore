import { useCallback, useEffect, useState } from "react";
import { api } from "../../api";
import { theme, radius } from "../../theme";
import type { EventGroup, EventLevel, EventSummary } from "../../types";
import { filterMatch, groupMatch, levelColor } from "./errorsMatch";

// R47.2: the System tab's errors & warnings. Repeats fold into one row
// (×N, first/last seen); Clear and per-row Dismiss only hide rows — the
// events stay in the audit trail and come back under "Show dismissed".

const POLL_MS = 10000;
const LEVELS: EventLevel[] = ["error", "warning", "info"];
const DETAIL_PREVIEW = 200;

function seenLine(g: EventGroup): string {
  const last = new Date(g.last_seen).toLocaleString();
  return g.count > 1 ? `${last} · first ${new Date(g.first_seen).toLocaleString()}` : last;
}

function EventRow({ group, onToggle, busy }: { group: EventGroup; onToggle: (g: EventGroup) => void; busy: boolean }) {
  const [open, setOpen] = useState(false);
  const long = group.detail.length > DETAIL_PREVIEW;
  return (
    <li style={{ ...styles.row, opacity: group.dismissed ? 0.6 : 1 }}>
      <div style={{ display: "flex", gap: 8, alignItems: "baseline", flexWrap: "wrap" }}>
        <span style={{ fontSize: 11, fontFamily: theme.mono, textTransform: "uppercase", color: levelColor(group.level) }}>{group.level}</span>
        {group.count > 1 && (
          <span style={styles.badge} title={`${group.count} occurrences`}>
            ×{group.count}
          </span>
        )}
        <span style={{ fontSize: 12, color: theme.textMuted }}>{seenLine(group)}</span>
        <span style={styles.tag}>{group.component}</span>
        {group.source_name && <span style={{ fontSize: 12, color: theme.accentAlt }}>{group.source_name}</span>}
        <button style={{ ...styles.small, marginLeft: "auto" }} disabled={busy} onClick={() => onToggle(group)}>
          {group.dismissed ? "Restore" : "Dismiss"}
        </button>
      </div>
      <div style={{ fontSize: 13 }}>{group.message}</div>
      {group.detail && (
        <div style={{ fontSize: 12, color: theme.textMuted, whiteSpace: open ? "pre-wrap" : undefined, wordBreak: "break-word" }}>
          {open || !long ? group.detail : `${group.detail.slice(0, DETAIL_PREVIEW)}…`}{" "}
          {long && (
            <button style={styles.linkButton} onClick={() => setOpen((o) => !o)}>
              {open ? "Less" : "More"}
            </button>
          )}
        </div>
      )}
    </li>
  );
}

export function ErrorsPanel({ refreshKey = 0 }: { refreshKey?: number }) {
  const [levels, setLevels] = useState<EventLevel[]>(["error", "warning"]);
  const [component, setComponent] = useState("");
  const [sourceId, setSourceId] = useState<number | null>(null);
  const [showDismissed, setShowDismissed] = useState(false);
  const [groups, setGroups] = useState<EventGroup[]>([]);
  const [summary, setSummary] = useState<EventSummary | null>(null);
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState<string | null>(null);

  const load = useCallback(() => {
    api
      .systemErrorGroups({ levels, component: component || undefined, sourceId: sourceId ?? undefined, includeDismissed: showDismissed, limit: 100 })
      .then(setGroups)
      .catch(() => {});
    api.systemErrorSummary().then(setSummary).catch(() => {});
  }, [levels, component, sourceId, showDismissed]);

  useEffect(() => {
    load();
    const timer = setInterval(load, POLL_MS);
    return () => clearInterval(timer);
  }, [load, refreshKey]);

  const run = async (action: () => Promise<string>) => {
    setBusy(true);
    try {
      setNote(await action());
    } catch (e) {
      setNote(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
      load();
    }
  };

  const toggleLevel = (l: EventLevel) => setLevels((cur) => (cur.includes(l) ? cur.filter((x) => x !== l) : [...cur, l]));
  const toggleRow = (g: EventGroup) =>
    run(async () => {
      if (g.dismissed) {
        const r = await api.restoreErrors(groupMatch(g));
        return `Restored ${r.restored}.`;
      }
      const r = await api.dismissErrors(groupMatch(g));
      return `Dismissed ${r.dismissed}.`;
    });
  const clearShown = () =>
    run(async () => {
      const r = await api.dismissErrors(filterMatch(levels, component, sourceId));
      return r.dismissed ? `Cleared ${r.dismissed}. Find them again under "Show dismissed".` : "Nothing to clear.";
    });

  const visible = groups.filter((g) => showDismissed || !g.dismissed);
  const anyShown = visible.some((g) => !g.dismissed);

  return (
    <section aria-label="Errors and warnings" style={{ marginBottom: 20 }}>
      <h3 style={{ marginBottom: 8 }}>Errors & warnings</h3>
      <div style={{ display: "flex", gap: 6, flexWrap: "wrap", alignItems: "center", marginBottom: 8 }}>
        {LEVELS.map((l) => {
          const on = levels.includes(l);
          return (
            <button
              key={l}
              aria-pressed={on}
              onClick={() => toggleLevel(l)}
              style={{ ...styles.chip, color: on ? levelColor(l) : theme.textDim, borderColor: on ? levelColor(l) : theme.border }}
            >
              {l} {summary?.levels[l] ?? 0}
            </button>
          );
        })}
        <select aria-label="Component" value={component} onChange={(e) => setComponent(e.target.value)} style={styles.select}>
          <option value="">All components</option>
          {(summary?.components ?? []).map((c) => (
            <option key={c.component} value={c.component}>
              {c.component} ({c.count})
            </option>
          ))}
        </select>
        {(summary?.sources.length ?? 0) > 0 && (
          <select
            aria-label="Source"
            value={sourceId ?? ""}
            onChange={(e) => setSourceId(e.target.value ? Number(e.target.value) : null)}
            style={styles.select}
          >
            <option value="">All sources</option>
            {summary!.sources.map((s) => (
              <option key={s.source_id} value={s.source_id}>
                {s.source_name ?? `source ${s.source_id}`} ({s.count})
              </option>
            ))}
          </select>
        )}
        <label style={{ fontSize: 12, color: theme.textMuted, display: "flex", gap: 4, alignItems: "center" }}>
          <input type="checkbox" checked={showDismissed} onChange={(e) => setShowDismissed(e.target.checked)} />
          Show dismissed ({summary?.dismissed ?? 0})
        </label>
        <button style={styles.small} disabled={busy || !anyShown || levels.length === 0} onClick={clearShown}>
          Clear shown
        </button>
      </div>
      {note && <div style={{ fontSize: 12, color: theme.textMuted, marginBottom: 6 }}>{note}</div>}
      {levels.length === 0 ? (
        <p style={{ color: theme.textDim, fontSize: 13 }}>Pick a level to see events.</p>
      ) : visible.length === 0 ? (
        <p style={{ color: theme.textDim, fontSize: 13 }}>No errors recorded.</p>
      ) : (
        <ul style={{ listStyle: "none", padding: 0, margin: 0, display: "grid", gap: 6 }}>
          {visible.map((g) => (
            <EventRow key={`${g.id}-${g.dismissed}`} group={g} onToggle={toggleRow} busy={busy} />
          ))}
        </ul>
      )}
    </section>
  );
}

const styles: Record<string, React.CSSProperties> = {
  row: { display: "flex", flexDirection: "column", gap: 2, background: theme.bgCard, border: `1px solid ${theme.border}`, borderRadius: radius, padding: "0.5rem 0.75rem" },
  chip: { padding: "0.2rem 0.6rem", borderRadius: radius, border: "1px solid", background: theme.bgCard, cursor: "pointer", fontSize: 12, fontFamily: theme.mono, textTransform: "uppercase" },
  select: { padding: "0.3rem 0.5rem", borderRadius: radius, border: `1px solid ${theme.border}`, background: theme.bgCard, color: theme.text, fontSize: 12 },
  small: { padding: "0.25rem 0.7rem", borderRadius: radius, border: `1px solid ${theme.border}`, background: theme.bgHover, color: theme.text, cursor: "pointer", fontSize: 12 },
  badge: { fontSize: 11, fontFamily: theme.mono, background: theme.bgHover, borderRadius: radius, padding: "0 0.35rem" },
  tag: { fontSize: 12, background: theme.bgHover, borderRadius: radius, padding: "0 0.35rem" },
  linkButton: { background: "none", border: "none", padding: 0, color: theme.accentAlt, cursor: "pointer", fontSize: 12 },
};
