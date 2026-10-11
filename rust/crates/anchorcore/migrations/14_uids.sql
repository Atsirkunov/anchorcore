-- R25.1: stable uids (UUIDv7) for export / device migration (docs/plan-backup-sync.md §6.1).
-- ALTER TABLE ADD COLUMN is not idempotent in SQLite: `db.rs:ensure_uid_columns`
-- mirrors these statements for DBs whose migration version already advanced, and
-- the Rust boot path backfills NULL uids because SQL cannot generate UUIDv7.
ALTER TABLE sources ADD COLUMN uid TEXT;
ALTER TABLE ingested_items ADD COLUMN uid TEXT;
ALTER TABLE entities ADD COLUMN uid TEXT;
ALTER TABLE chunks ADD COLUMN uid TEXT;
ALTER TABLE sections ADD COLUMN uid TEXT;
ALTER TABLE tags ADD COLUMN uid TEXT;
ALTER TABLE projects ADD COLUMN uid TEXT;
CREATE UNIQUE INDEX IF NOT EXISTS ix_sources_uid ON sources(uid);
CREATE UNIQUE INDEX IF NOT EXISTS ix_ingested_items_uid ON ingested_items(uid);
CREATE UNIQUE INDEX IF NOT EXISTS ix_entities_uid ON entities(uid);
CREATE UNIQUE INDEX IF NOT EXISTS ix_chunks_uid ON chunks(uid);
CREATE UNIQUE INDEX IF NOT EXISTS ix_sections_uid ON sections(uid);
CREATE UNIQUE INDEX IF NOT EXISTS ix_tags_uid ON tags(uid);
CREATE UNIQUE INDEX IF NOT EXISTS ix_projects_uid ON projects(uid);
