-- Backfill created_at default for system_events on DBs created by older
-- chains (Python/alembic) without it. Without the default, every audit writer
-- that omits created_at fails (errors are swallowed at call sites), so the
-- audit trail silently goes missing. Recreate preserves all rows.
CREATE TABLE IF NOT EXISTS system_events_new (
    id INTEGER PRIMARY KEY,
    component VARCHAR(50) NOT NULL,
    level VARCHAR(20) NOT NULL DEFAULT 'error',
    source_id INTEGER REFERENCES sources(id) ON DELETE SET NULL,
    message TEXT NOT NULL DEFAULT '',
    detail TEXT NOT NULL DEFAULT '',
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);
INSERT OR IGNORE INTO system_events_new (id, component, level, source_id, message, detail, created_at)
    SELECT id, component, level, source_id, message, detail, created_at FROM system_events;
DROP TABLE IF EXISTS system_events;
ALTER TABLE system_events_new RENAME TO system_events;
CREATE INDEX IF NOT EXISTS ix_system_events_component ON system_events(component);
