import { useEffect, useState } from "react";
import { api } from "./api";
import { commonStyles, theme } from "./theme";
import type { FolderListing } from "./types";

function message(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/**
 * Folder browser for folder sources (Sources add/edit form + onboarding
 * wizard). The server lists its own directories (`GET /system/fs`) because a
 * browser cannot hand an absolute path back to a local server — native
 * directory pickers return opaque handles, not paths.
 *
 * Navigation: click a folder to open it, type/paste an absolute path and press
 * Enter, or use Up / Home / Roots. "Use this folder" picks the current one.
 */
export function FolderPicker({
  initialPath,
  onSelect,
  onCancel,
}: {
  initialPath?: string;
  onSelect: (path: string) => void;
  onCancel: () => void;
}) {
  // Captured once: edits in the parent's path input while the picker is open
  // must not reset the navigation.
  const [initial] = useState(() => (initialPath ?? "").trim());
  const [listing, setListing] = useState<FolderListing | null>(null);
  const [pathInput, setPathInput] = useState("");
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  async function go(path?: string) {
    setLoading(true);
    setError(null);
    try {
      const res = await api.browseFolders(path);
      setListing(res);
      setPathInput(res.path ?? "");
    } catch (e) {
      setError(message(e));
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    let cancelled = false;
    const apply = (res: FolderListing) => {
      if (cancelled) return;
      setListing(res);
      setPathInput(res.path ?? "");
    };
    (async () => {
      if (initial) {
        try {
          apply(await api.browseFolders(initial));
          setLoading(false);
          return;
        } catch {
          if (cancelled) return;
          setError(`"${initial}" is not a folder — pick one below or type an absolute path.`);
        }
      }
      try {
        apply(await api.browseFolders());
      } catch (e) {
        if (!cancelled) setError(message(e));
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [initial]);

  const dirs = listing?.dirs ?? [];

  return (
    <div
      style={styles.panel}
      onKeyDown={(e) => {
        if (e.key === "Escape") onCancel();
      }}
    >
      <div style={styles.row}>
        <button
          type="button"
          style={styles.navButton}
          disabled={!listing?.parent || loading}
          title="Parent folder"
          onClick={() => listing?.parent && go(listing.parent)}
        >
          ↑ Up
        </button>
        <input
          style={styles.pathInput}
          value={pathInput}
          placeholder="Absolute folder path"
          spellCheck={false}
          onChange={(e) => setPathInput(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") go(pathInput.trim() || undefined);
            if (e.key === "Escape") onCancel();
          }}
        />
        <button type="button" style={styles.navButton} disabled={!listing?.home || loading} title="Your home folder" onClick={() => listing?.home && go(listing.home)}>
          Home
        </button>
        <button type="button" style={styles.navButton} disabled={loading} title="Drives and home folders" onClick={() => go()}>
          Roots
        </button>
      </div>

      {error && <div style={styles.error}>{error}</div>}

      <div style={styles.list}>
        {loading && !listing ? (
          <div style={styles.hint}>Loading…</div>
        ) : (
          <>
            {dirs.map((d) => (
              <button key={d.path} type="button" style={styles.dir} title={d.path} disabled={loading} onClick={() => go(d.path)}>
                📁 {d.name}
              </button>
            ))}
            {dirs.length === 0 && <div style={styles.hint}>No subfolders — you can select this folder.</div>}
            {listing?.truncated && <div style={styles.hint}>First {dirs.length} folders shown.</div>}
          </>
        )}
      </div>

      <div style={styles.footer}>
        <span style={styles.current}>{listing?.path ? `Current: ${listing.path}` : "Pick a drive or Home to start"}</span>
        <div style={{ display: "flex", gap: 8, flexShrink: 0 }}>
          <button
            type="button"
            style={styles.primary}
            disabled={!listing?.path || loading}
            onClick={() => listing?.path && onSelect(listing.path)}
          >
            Use this folder
          </button>
          <button type="button" style={styles.secondary} onClick={onCancel}>
            Cancel
          </button>
        </div>
      </div>
    </div>
  );
}

const styles: Record<string, React.CSSProperties> = {
  panel: {
    display: "grid",
    gap: 8,
    background: theme.bgElevated,
    border: `1px solid ${theme.border}`,
    borderRadius: 8,
    padding: "0.6rem 0.75rem",
  },
  row: { display: "flex", gap: 8, alignItems: "center" },
  navButton: {
    ...commonStyles.button,
    padding: "0.35rem 0.6rem",
    fontSize: 12,
    flexShrink: 0,
  },
  pathInput: {
    ...commonStyles.input,
    flex: 1,
    minWidth: 120,
    padding: "0.35rem 0.6rem",
    fontSize: 12,
  },
  error: { fontSize: 12, color: theme.red },
  list: {
    display: "grid",
    gap: 3,
    maxHeight: 200,
    overflowY: "auto",
    border: `1px solid ${theme.borderSoft}`,
    borderRadius: 6,
    background: theme.bg,
    padding: 4,
  },
  dir: {
    textAlign: "left",
    background: theme.bgCard,
    border: `1px solid ${theme.borderSoft}`,
    borderRadius: 6,
    color: theme.text,
    padding: "0.35rem 0.6rem",
    fontSize: 13,
    cursor: "pointer",
    overflow: "hidden",
    textOverflow: "ellipsis",
    whiteSpace: "nowrap",
  },
  hint: { fontSize: 12, color: theme.textDim, padding: "0.25rem 0.35rem" },
  footer: { display: "flex", gap: 8, alignItems: "center", justifyContent: "space-between" },
  current: { fontSize: 12, color: theme.textMuted, wordBreak: "break-all" },
  primary: {
    ...commonStyles.button,
    background: theme.accent,
    color: theme.onAccent,
    padding: "0.35rem 0.8rem",
    fontSize: 13,
  },
  secondary: {
    ...commonStyles.button,
    padding: "0.35rem 0.8rem",
    fontSize: 13,
  },
};
