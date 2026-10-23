import { useCallback, useEffect, useState } from "react";
import { api } from "../../api";
import { Thumb } from "../../ImageStrip";
import { theme, commonStyles, radius } from "../../theme";
import type { ItemMeta, MediaRow, Source } from "../../types";

type View = "none" | "ignored";

const PAGE = 60;

/** R46.12: images sync could not place (no synced document links them).
 *  A person writes the caption and picks the document it belongs to; that
 *  caption is how search finds the image. People only (owner decision
 *  2026-10-23): nothing here suggests a caption. */
export function UnplacedImages({ sources }: { sources: Source[] }) {
  const [view, setView] = useState<View>("none");
  const [rows, setRows] = useState<MediaRow[]>([]);
  const [total, setTotal] = useState(0);
  const [counts, setCounts] = useState({ none: 0, ignored: 0 });
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(() => {
    setError(null);
    api
      .media({ attribution: view, limit: PAGE })
      .then((list) => {
        setRows(list.items);
        setTotal(list.total);
      })
      .catch((e) => setError(e instanceof Error ? e.message : String(e)));
    // Both counts: the badge, and whether the panel shows at all (ignored
    // images must stay restorable when nothing is left unplaced).
    Promise.all([api.media({ attribution: "none", limit: 1 }), api.media({ attribution: "ignored", limit: 1 })])
      .then(([n, i]) => setCounts({ none: n.total, ignored: i.total }))
      .catch(() => {});
  }, [view]);

  // `sources` changes identity on every Sources refresh (sync finished).
  useEffect(load, [load, sources]);

  if (counts.none === 0 && counts.ignored === 0 && !error) return null;
  const unplaced = counts.none;

  return (
    <section style={styles.panel} aria-labelledby="unplaced-heading">
      <div style={styles.header}>
        <span id="unplaced-heading" style={{ fontWeight: 600 }}>
          Unplaced images
        </span>
        <span style={styles.badge} aria-label={`${unplaced} unplaced`}>
          {unplaced}
        </span>
        <span style={{ flex: 1 }} />
        <button style={view === "none" ? styles.tabOn : styles.tab} onClick={() => setView("none")}>
          Unplaced
        </button>
        <button style={view === "ignored" ? styles.tabOn : styles.tab} onClick={() => setView("ignored")}>
          Ignored
        </button>
      </div>
      <p style={styles.help}>
        No synced document links these images. Write what each one shows and pick the document it belongs to — your caption is
        how questions find it. Nothing is suggested automatically.
      </p>
      {error && <p style={{ color: theme.red, margin: "0 0 8px" }}>{error}</p>}
      {rows.length === 0 ? (
        <div style={{ fontSize: 12, color: theme.textDim }}>{view === "none" ? "Every image is placed." : "No ignored images."}</div>
      ) : (
        <div style={styles.grid}>
          {rows.map((r) => (
            <ImageCard key={r.asset_id} row={r} onDone={load} />
          ))}
        </div>
      )}
      {total > rows.length && (
        <div style={{ fontSize: 12, color: theme.textDim, marginTop: 8 }}>
          Showing {rows.length} of {total}.
        </div>
      )}
    </section>
  );
}

function ImageCard({ row, onDone }: { row: MediaRow; onDone: () => void }) {
  const [caption, setCaption] = useState(row.manual_caption);
  const [target, setTarget] = useState<number | "">(row.target?.item_id ?? "");
  const [docs, setDocs] = useState<ItemMeta[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const ignored = row.attribution === "ignored";

  function loadDocs() {
    if (docs) return;
    api
      .listItems({ source_id: row.source_id, limit: 200 })
      .then((l) => setDocs(l.items.filter((i) => i.id !== row.item_id)))
      .catch(() => setDocs([]));
  }

  async function patch(body: Parameters<typeof api.patchMedia>[1]) {
    setBusy(true);
    setError(null);
    try {
      await api.patchMedia(row.asset_id, body);
      onDone();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  const save = () => patch({ caption: caption.trim(), target: target === "" ? null : { item_id: target } });
  const canSave = caption.trim() !== "" || target !== "";

  return (
    <div style={styles.card}>
      <Thumb a={{ asset_id: row.asset_id, alt: row.alt, caption: row.manual_caption || row.caption, mime: row.mime, w: row.w, h: row.h, url: row.url }} />
      <div style={styles.path} title={`${row.source_name} · ${row.path}`}>
        {row.path}
      </div>
      <div style={{ fontSize: 11, color: theme.textDim }}>{row.source_name}</div>
      {ignored ? (
        <button style={styles.small} disabled={busy} onClick={() => patch({ ignore: false })}>
          Restore
        </button>
      ) : (
        <>
          <label style={styles.label}>
            What it shows
            <textarea
              style={{ ...commonStyles.input, minHeight: 56, resize: "vertical", fontFamily: theme.sans, fontSize: 13 }}
              value={caption}
              maxLength={2000}
              placeholder="e.g. Checkout flow: cart → payment → receipt"
              onChange={(e) => setCaption(e.target.value)}
            />
          </label>
          <label style={styles.label}>
            Belongs to
            <select
              style={{ ...commonStyles.input, fontSize: 13 }}
              value={target}
              onFocus={loadDocs}
              onMouseDown={loadDocs}
              onChange={(e) => setTarget(e.target.value === "" ? "" : Number(e.target.value))}
            >
              <option value="">This image on its own</option>
              {row.target && !docs && <option value={row.target.item_id}>{row.target.item_title}</option>}
              {(docs ?? []).map((d) => (
                <option key={d.id} value={d.id}>
                  {d.path || d.title}
                </option>
              ))}
            </select>
          </label>
          {error && <div style={{ color: theme.red, fontSize: 12 }}>{error}</div>}
          <div style={{ display: "flex", gap: 6 }}>
            <button style={{ ...styles.small, ...styles.primary }} disabled={busy || !canSave} onClick={save}>
              Save
            </button>
            <button style={styles.small} disabled={busy} onClick={() => patch({ ignore: true })} title="Hide from answers without deleting">
              Ignore
            </button>
          </div>
        </>
      )}
    </div>
  );
}

const styles: Record<string, React.CSSProperties> = {
  panel: { background: theme.bgElevated, border: `1px solid ${theme.border}`, borderRadius: radius, padding: "0.9rem 1rem", marginBottom: 16 },
  header: { display: "flex", alignItems: "center", gap: 8, marginBottom: 4 },
  badge: { fontFamily: theme.mono, fontSize: 11, padding: "0 6px", borderRadius: radius, background: theme.amberBg, color: theme.text, border: `1px solid ${theme.border}` },
  tab: { ...commonStyles.button, fontSize: 12, padding: "0.2rem 0.6rem" },
  tabOn: { ...commonStyles.button, fontSize: 12, padding: "0.2rem 0.6rem", background: theme.accent, color: theme.onAccent },
  help: { fontSize: 12, color: theme.textDim, margin: "0 0 10px" },
  grid: { display: "grid", gridTemplateColumns: "repeat(auto-fill, minmax(220px, 1fr))", gap: 10 },
  card: { display: "grid", gap: 6, alignContent: "start", background: theme.bgCard, border: `1px solid ${theme.border}`, borderRadius: radius, padding: 8, minWidth: 0 },
  path: { fontFamily: theme.mono, fontSize: 11, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" },
  label: { display: "grid", gap: 3, fontSize: 11, color: theme.textDim },
  small: { ...commonStyles.button, fontSize: 12, padding: "0.25rem 0.6rem" },
  primary: { background: theme.accent, color: theme.onAccent },
};
