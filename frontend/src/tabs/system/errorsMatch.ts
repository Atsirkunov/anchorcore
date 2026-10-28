import { theme } from "../../theme";
import type { EventGroup, EventLevel, EventMatch } from "../../types";

export const levelColor = (level: string) => (level === "error" ? theme.red : level === "warning" ? theme.amber : theme.blue);

/** The dismiss/restore match for one grouped row: exactly its own events. */
export function groupMatch(g: EventGroup): EventMatch {
  return { component: g.component, levels: [g.level], source_id: g.source_id, message: g.message };
}

/** The match for "Clear shown": the active filters, nothing more. */
export function filterMatch(levels: EventLevel[], component: string, sourceId: number | null): EventMatch {
  const m: EventMatch = { levels };
  if (component) m.component = component;
  if (sourceId != null) m.source_id = sourceId;
  return m;
}
