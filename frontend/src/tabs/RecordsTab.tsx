import { useState } from "react";
import { theme, radius } from "../theme";
import { ActionsView, DecisionsView } from "./RecordsViews";
import { GlossaryPanel } from "./GlossaryPanel";

/** R45.25: the Records tab — the decision log, the action tracker and the
 *  glossary. The old "All entities" review queue is gone: nothing reads
 *  `verified`, so confirming rows changed nothing (decision 2026-10-30).
 *  `/entities*` and `/review/*` stay for REST and MCP callers. */
type RecordsView = "decisions" | "actions" | "glossary";
// Kept from the Entities tab so a remembered Decisions/Actions choice survives;
// its retired "all" value falls back to Decisions.
const VIEW_KEY = "entities-view";
const VIEWS: { id: RecordsView; label: string }[] = [
  { id: "decisions", label: "Decisions" },
  { id: "actions", label: "Actions" },
  { id: "glossary", label: "Glossary" },
];

function readView(): RecordsView {
  try {
    const v = localStorage.getItem(VIEW_KEY);
    return v === "actions" || v === "glossary" ? v : "decisions";
  } catch {
    return "decisions";
  }
}

export function RecordsTab() {
  const [view, setViewState] = useState<RecordsView>(readView);
  function setView(v: RecordsView) {
    setViewState(v);
    try {
      localStorage.setItem(VIEW_KEY, v);
    } catch {
      // per-viewer convenience only
    }
  }
  return (
    <div>
      <div style={styles.viewSwitch}>
        {VIEWS.map((v) => (
          <button key={v.id} onClick={() => setView(v.id)} style={{ ...styles.toggle, ...(view === v.id ? styles.toggleActive : {}) }}>
            {v.label}
          </button>
        ))}
      </div>
      {view === "decisions" && <DecisionsView />}
      {view === "actions" && <ActionsView />}
      {view === "glossary" && <GlossaryPanel defaultOpen />}
    </div>
  );
}

const styles: Record<string, React.CSSProperties> = {
  viewSwitch: { display: "flex", gap: 4, marginBottom: 12, border: `1px solid ${theme.border}`, borderRadius: radius, padding: 2, background: theme.bgCard, width: "fit-content" },
  toggle: { background: "none", border: "none", color: theme.textDim, padding: "0.2rem 0.6rem", borderRadius: radius, cursor: "pointer", fontSize: 12, fontWeight: 600 },
  toggleActive: { background: theme.accent, color: theme.onAccent },
};
