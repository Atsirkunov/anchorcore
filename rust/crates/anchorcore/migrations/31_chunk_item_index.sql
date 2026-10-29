-- R41.10: `chunks.item_id` had no index, so every per-item chunk lookup
-- scanned the whole table. On a multi-GB library the R36.2 re-chunk check
-- that ends every boot (one lookup per item) ran for an hour or more behind
-- "Checking the index", and each re-chunked item scanned it several times
-- more. `assets.chunk_id` is the `ON DELETE SET NULL` child of `chunks`: with
-- foreign keys on, every deleted chunk looked it up the same way.
CREATE INDEX IF NOT EXISTS ix_chunks_item_id ON chunks(item_id);
CREATE INDEX IF NOT EXISTS ix_assets_chunk_id ON assets(chunk_id);
