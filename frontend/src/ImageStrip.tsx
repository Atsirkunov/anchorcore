import { useEffect, useState } from "react";
import { api } from "./api";
import { theme, radius } from "./theme";
import type { AssetRef } from "./types";

/** Thumbnail height; width follows the image's own aspect ratio (from the
 *  stored w/h, so nothing shifts while the bytes load). */
const THUMB_H = 88;
const MIN_W = 56;
const MAX_W = 200;

function thumbWidth(a: AssetRef): number {
  if (!a.w || !a.h) return THUMB_H;
  return Math.round(Math.min(MAX_W, Math.max(MIN_W, (THUMB_H * a.w) / a.h)));
}

function label(a: AssetRef): string {
  return a.caption || a.alt || "image";
}

function LockGlyph() {
  return (
    <svg width="16" height="16" viewBox="0 0 16 16" aria-hidden="true" fill="none" stroke="currentColor" strokeWidth="1.4">
      <rect x="3" y="7" width="10" height="7" rx="1.5" />
      <path d="M5.5 7V5a2.5 2.5 0 0 1 5 0v2" />
    </svg>
  );
}

/** One thumbnail. Bytes come from `GET /media/:uid` (DB only); a null url
 *  (not stored — over the size cap) or a failed load (403 gated_provider,
 *  missing) renders a labelled lock tile, never a broken image. */
function Thumb({ a }: { a: AssetRef }) {
  const [failed, setFailed] = useState(false);
  const width = thumbWidth(a);
  const blocked = !a.url || failed;
  const reason = !a.url ? "Not stored (over the size cap)" : "Withheld or unavailable";
  return (
    <figure style={{ ...styles.figure, width }}>
      {blocked ? (
        <div style={{ ...styles.tile, ...styles.locked, width, height: THUMB_H }} title={`${label(a)} — ${reason}`} role="img" aria-label={`${label(a)} (${reason})`}>
          <LockGlyph />
          <span style={styles.lockText}>{!a.url ? "not stored" : "withheld"}</span>
        </div>
      ) : (
        <a href={a.url ?? undefined} target="_blank" rel="noopener noreferrer" title={`Open ${label(a)}`}>
          <img
            src={a.url ?? undefined}
            alt={a.alt || a.caption || "image"}
            loading="lazy"
            width={width}
            height={THUMB_H}
            onError={() => setFailed(true)}
            style={{ ...styles.tile, width, height: THUMB_H }}
          />
        </a>
      )}
      <figcaption style={styles.caption} title={label(a)}>
        {label(a)}
      </figcaption>
    </figure>
  );
}

/** R46.11: a row of image thumbnails (source cards, document drill-down). */
export function ImageStrip({ images }: { images: AssetRef[] }) {
  if (!images.length) return null;
  return (
    <div style={styles.strip}>
      {images.map((a) => (
        <Thumb key={a.asset_id} a={a} />
      ))}
    </div>
  );
}

/** The images of one item (GET /items/:id `assets`), fetched on mount. */
export function ItemImages({ itemId }: { itemId: number }) {
  const [images, setImages] = useState<AssetRef[] | null>(null);
  useEffect(() => {
    let live = true;
    api
      .item(itemId)
      .then((it) => live && setImages(it.assets ?? []))
      .catch(() => live && setImages([]));
    return () => {
      live = false;
    };
  }, [itemId]);
  if (!images || images.length === 0) return null;
  return (
    <div style={{ marginBottom: 8 }}>
      <div style={styles.heading}>Images · {images.length}</div>
      <ImageStrip images={images} />
    </div>
  );
}

const styles: Record<string, React.CSSProperties> = {
  strip: { display: "flex", flexWrap: "wrap", gap: 8, marginTop: 8 },
  figure: { margin: 0, display: "grid", gap: 3 },
  tile: { display: "block", objectFit: "contain", background: theme.bgElevated, border: `1px solid ${theme.border}`, borderRadius: radius },
  locked: { display: "flex", flexDirection: "column", alignItems: "center", justifyContent: "center", gap: 4, color: theme.textDim },
  lockText: { fontFamily: theme.mono, fontSize: 10, textTransform: "uppercase", letterSpacing: "0.05em" },
  caption: { fontSize: 11, lineHeight: 1.35, color: theme.textDim, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" },
  heading: { fontSize: 10, color: theme.textDim, textTransform: "uppercase", marginBottom: 2 },
};
