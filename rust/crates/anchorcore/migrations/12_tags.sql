-- R14.4 dynamic tag taxonomy
CREATE TABLE IF NOT EXISTS tags (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    description TEXT NOT NULL DEFAULT '',
    embedding BLOB,
    count INTEGER NOT NULL DEFAULT 1,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX IF NOT EXISTS ix_tags_name ON tags(name);
CREATE TABLE IF NOT EXISTS chunk_tags (
    chunk_id INTEGER NOT NULL REFERENCES chunks(id) ON DELETE CASCADE,
    tag_id INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    PRIMARY KEY (chunk_id, tag_id)
);
CREATE INDEX IF NOT EXISTS ix_chunk_tags_tag_id ON chunk_tags(tag_id);
