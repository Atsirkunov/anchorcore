CREATE TABLE IF NOT EXISTS system_events (
    id INTEGER PRIMARY KEY,
    component VARCHAR(50) NOT NULL,
    level VARCHAR(20) NOT NULL DEFAULT 'error',
    source_id INTEGER REFERENCES sources(id) ON DELETE SET NULL,
    message TEXT NOT NULL DEFAULT '',
    detail TEXT NOT NULL DEFAULT '',
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX IF NOT EXISTS ix_system_events_component ON system_events(component);
