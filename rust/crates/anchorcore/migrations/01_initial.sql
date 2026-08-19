-- 23bca0413318_initial_schema (idempotent)
CREATE TABLE IF NOT EXISTS sources (
    id INTEGER PRIMARY KEY,
    connector VARCHAR(50) NOT NULL,
    name VARCHAR(300) NOT NULL,
    config TEXT NOT NULL DEFAULT '{}',
    last_sync_cursor VARCHAR(500) NOT NULL DEFAULT '',
    last_synced_at DATETIME,
    last_error TEXT,
    error_count INTEGER NOT NULL DEFAULT 0,
    enabled BOOLEAN NOT NULL DEFAULT 1,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE TABLE IF NOT EXISTS ingested_items (
    id INTEGER PRIMARY KEY,
    source_id INTEGER NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
    external_id VARCHAR(500) NOT NULL DEFAULT '',
    title VARCHAR(500) NOT NULL DEFAULT '',
    text TEXT NOT NULL DEFAULT '',
    content_hash VARCHAR(64) NOT NULL,
    author VARCHAR(300) NOT NULL DEFAULT '',
    updated_at DATETIME,
    stale BOOLEAN NOT NULL DEFAULT 0,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX IF NOT EXISTS ix_ingested_items_content_hash ON ingested_items(content_hash);
CREATE TABLE IF NOT EXISTS entities (
    id INTEGER PRIMARY KEY,
    item_id INTEGER NOT NULL REFERENCES ingested_items(id) ON DELETE CASCADE,
    kind VARCHAR(50) NOT NULL,
    summary TEXT NOT NULL DEFAULT '',
    reasoning TEXT NOT NULL DEFAULT '',
    confidence FLOAT NOT NULL DEFAULT 0.0,
    author VARCHAR(300) NOT NULL DEFAULT '',
    source_ref VARCHAR(500) NOT NULL DEFAULT '',
    status VARCHAR(50) NOT NULL DEFAULT 'unverified',
    owner VARCHAR(300) NOT NULL DEFAULT '',
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX IF NOT EXISTS ix_entities_kind ON entities(kind);
CREATE INDEX IF NOT EXISTS ix_entities_status ON entities(status);
CREATE TABLE IF NOT EXISTS chunks (
    id INTEGER PRIMARY KEY,
    item_id INTEGER REFERENCES ingested_items(id) ON DELETE CASCADE,
    entity_id INTEGER REFERENCES entities(id) ON DELETE CASCADE,
    source_ref VARCHAR(500) NOT NULL DEFAULT '',
    content TEXT NOT NULL DEFAULT '',
    embedding BLOB,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE TABLE IF NOT EXISTS merge_actions (
    id INTEGER PRIMARY KEY,
    entity_a_id INTEGER NOT NULL REFERENCES entities(id) ON DELETE CASCADE,
    entity_b_id INTEGER REFERENCES entities(id) ON DELETE CASCADE,
    status VARCHAR(50) NOT NULL DEFAULT 'proposed',
    reason VARCHAR(500) NOT NULL DEFAULT '',
    user VARCHAR(300) NOT NULL DEFAULT '',
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE TABLE IF NOT EXISTS relationships (
    id INTEGER PRIMARY KEY,
    from_entity_id INTEGER NOT NULL REFERENCES entities(id) ON DELETE CASCADE,
    to_entity_id INTEGER NOT NULL REFERENCES entities(id) ON DELETE CASCADE,
    kind VARCHAR(100) NOT NULL DEFAULT 'related',
    source_ref VARCHAR(500) NOT NULL DEFAULT '',
    confidence FLOAT NOT NULL DEFAULT 0.0,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);
