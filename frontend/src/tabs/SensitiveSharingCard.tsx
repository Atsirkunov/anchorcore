import { useCallback, useEffect, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { api } from "../api";
import { destinationStatus, SHARING_QUERY_KEY } from "../sharingText";
import { radius, theme } from "../theme";
import type { SensitiveSharing, SharingDestination, SharingDestinationId } from "../types";


const ORDER: SharingDestinationId[] = ["embed", "answer", "agents"];

/** R35.6: the "Sensitive sources" card — one switch per destination, off by
 * default, each naming what it allows. Changes apply at once (no Save) and
 * are audited server-side. `refreshKey` re-reads after the model settings
 * are saved (a new provider host closes its switch). */
export function SensitiveSharingCard({ refreshKey }: { refreshKey: number }) {
  const [state, setState] = useState<SensitiveSharing | null>(null);
  const [busy, setBusy] = useState<SharingDestinationId | null>(null);
  const [error, setError] = useState<string | null>(null);
  const queryClient = useQueryClient();

  const load = useCallback(() => {
    api
      .getSensitiveSharing()
      .then(setState)
      .catch((e) => setError(e instanceof Error ? e.message : String(e)));
  }, []);

  useEffect(() => load(), [load, refreshKey]);

  async function toggle(id: SharingDestinationId, on: boolean) {
    setBusy(id);
    setError(null);
    try {
      const next = await api.updateSensitiveSharing({ [id]: on });
      setState(next);
      queryClient.setQueryData(SHARING_QUERY_KEY, next);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  }

  return (
    <div style={styles.section}>
      <div style={styles.sectionTitle}>Sensitive sources</div>
      <div style={{ fontSize: 12, color: theme.textDim, lineHeight: 1.5 }}>
        Content from Sensitive sources (and documents marked Sensitive) stays with local models and this app by default.
        Each switch allows one destination to receive it. You see everything here either way.
      </div>
      {!state && !error && <div style={{ fontSize: 12, marginTop: 8 }}>Loading…</div>}
      {state && ORDER.map((id) => <DestinationRow key={id} d={state[id]} busy={busy === id} onToggle={toggle} />)}
      {error && <div style={{ color: theme.red, fontSize: 12, marginTop: 8 }}>{error}</div>}
    </div>
  );
}

function DestinationRow({ d, busy, onToggle }: { d: SharingDestination; busy: boolean; onToggle: (id: SharingDestinationId, on: boolean) => void }) {
  const fixed = d.local || d.env;
  return (
    <label style={{ display: "flex", gap: 8, alignItems: "flex-start", marginTop: 10, fontSize: 13, cursor: fixed ? "default" : "pointer" }}>
      <input
        type="checkbox"
        aria-label={d.label}
        checked={d.allowed}
        disabled={fixed || busy}
        onChange={(e) => onToggle(d.id, e.target.checked)}
      />
      <span>
        {d.label}
        <span style={{ display: "block", fontSize: 11, color: theme.textMuted, overflowWrap: "anywhere" }}>
          Allows: {d.destination}
        </span>
        <span style={{ display: "block", fontSize: 11, color: d.allowed && !d.local ? theme.amber : theme.textMuted }}>
          {destinationStatus(d)}
        </span>
      </span>
    </label>
  );
}

const styles: Record<string, React.CSSProperties> = {
  section: { background: theme.bgCard, border: `1px solid ${theme.border}`, borderRadius: radius, padding: "0.9rem 1rem" },
  sectionTitle: { fontWeight: 600, fontSize: 14, marginBottom: 4 },
};
