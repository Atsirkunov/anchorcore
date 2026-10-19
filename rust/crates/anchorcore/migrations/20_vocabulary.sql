-- R40.7 corpus vocabulary: Tier-1 rule pairs (active immediately) +
-- quarantined Tier-2 LLM pairs (proposed until corroborated by a second
-- independent item, rule-confirmed elsewhere, or human-approved in R40.8
-- review — rejected pairs stay hidden and are never re-proposed).
CREATE TABLE IF NOT EXISTS vocabulary (
    id INTEGER PRIMARY KEY,
    term TEXT NOT NULL,
    expansion TEXT NOT NULL,
    item_id INTEGER REFERENCES ingested_items(id) ON DELETE CASCADE,
    source_id INTEGER REFERENCES sources(id) ON DELETE CASCADE,
    origin TEXT NOT NULL DEFAULT 'rule',
    status TEXT NOT NULL DEFAULT 'proposed',
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS ix_vocabulary_lookup ON vocabulary(status, source_id, term, expansion);
