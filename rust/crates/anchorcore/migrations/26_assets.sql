-- R46.7: image assets. Bytes live in the DB for every source kind, folders
-- included — never path references (owner decision 2026-10-23, Phase 46):
-- an export must restore every citable image after the source is gone.
-- Identical images share one `asset_blobs` row (content hash), so dedupe is
-- structural; the triggers drop a blob once no asset references it, which
-- also covers FK cascades (source/item delete → assets → blob).
CREATE TABLE IF NOT EXISTS asset_blobs (
    hash TEXT PRIMARY KEY,
    bytes BLOB NOT NULL,
    size INTEGER NOT NULL,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);
-- origin: 'embed' (an image a document references — markdown/HTML) |
-- 'file' (an image file item of its own). attribution: embedded |
-- referenced | contextual | manual | none | ignored (R46.12 owns the
-- manual_* fields; sync never overwrites them). `caption` is the source's
-- own caption (markdown/HTML title, <figcaption>); `context` is the hosting
-- chunk's breadcrumb + nearby text. bytes_hash NULL = over the size cap
-- (metadata kept, bytes not stored).
CREATE TABLE IF NOT EXISTS assets (
    id INTEGER PRIMARY KEY,
    uid TEXT NOT NULL UNIQUE,
    item_id INTEGER NOT NULL REFERENCES ingested_items(id) ON DELETE CASCADE,
    section_id INTEGER REFERENCES sections(id) ON DELETE SET NULL,
    chunk_id INTEGER REFERENCES chunks(id) ON DELETE SET NULL,
    origin TEXT NOT NULL,
    src TEXT NOT NULL DEFAULT '',
    ref_path TEXT NOT NULL DEFAULT '',
    mime TEXT NOT NULL DEFAULT '',
    w INTEGER,
    h INTEGER,
    bytes_hash TEXT REFERENCES asset_blobs(hash),
    size INTEGER NOT NULL DEFAULT 0,
    alt TEXT NOT NULL DEFAULT '',
    caption TEXT NOT NULL DEFAULT '',
    context TEXT NOT NULL DEFAULT '',
    attribution TEXT NOT NULL DEFAULT 'none',
    manual_caption TEXT NOT NULL DEFAULT '',
    manual_target TEXT NOT NULL DEFAULT '',
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX IF NOT EXISTS ix_assets_item_id ON assets(item_id);
CREATE INDEX IF NOT EXISTS ix_assets_bytes_hash ON assets(bytes_hash);
CREATE INDEX IF NOT EXISTS ix_assets_attribution ON assets(attribution);
CREATE TRIGGER IF NOT EXISTS assets_blob_gc_delete AFTER DELETE ON assets
WHEN OLD.bytes_hash IS NOT NULL
BEGIN
    DELETE FROM asset_blobs WHERE hash = OLD.bytes_hash
        AND NOT EXISTS (SELECT 1 FROM assets WHERE bytes_hash = OLD.bytes_hash);
END;
CREATE TRIGGER IF NOT EXISTS assets_blob_gc_update AFTER UPDATE OF bytes_hash ON assets
WHEN OLD.bytes_hash IS NOT NULL AND OLD.bytes_hash IS NOT NEW.bytes_hash
BEGIN
    DELETE FROM asset_blobs WHERE hash = OLD.bytes_hash
        AND NOT EXISTS (SELECT 1 FROM assets WHERE bytes_hash = OLD.bytes_hash);
END;
