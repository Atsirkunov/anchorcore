-- R14.1 hierarchical TOC: sections tree + chunk section cols (idempotent)
CREATE TABLE IF NOT EXISTS sections (
    id INTEGER PRIMARY KEY,
    item_id INTEGER NOT NULL REFERENCES ingested_items(id) ON DELETE CASCADE,
    parent_id INTEGER REFERENCES sections(id) ON DELETE SET NULL,
    level INTEGER NOT NULL DEFAULT 0,
    title TEXT NOT NULL DEFAULT '',
    path TEXT NOT NULL DEFAULT '',
    chunk_range TEXT NOT NULL DEFAULT '',
    summary TEXT NOT NULL DEFAULT '',
    summary_embedding BLOB,
    summary_hash TEXT NOT NULL DEFAULT '',
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX IF NOT EXISTS ix_sections_item_id ON sections(item_id);
CREATE INDEX IF NOT EXISTS ix_sections_parent_id ON sections(parent_id);
-- chunks additions for Phase 14 (idempotent via IF NOT EXISTS check in try_add_column, but keep here for fresh DB)
-- ALTERs are handled in ensure_columns for existing DBs; for fresh DB run they are already in the table via migration.
-- We add them conditionally via separate try_add in db.rs, but also ensure here for new DBs that may have old chunks table.
-- SQLite has no IF NOT EXISTS for ADD COLUMN before 3.35, so we rely on ensure_columns for existing DBs.
-- For fresh DBs with new schema, ensure_columns will create cols; this file only ensures sections table.
