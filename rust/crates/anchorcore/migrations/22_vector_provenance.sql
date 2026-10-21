-- R45.5: vector provenance. Every stored chunk vector carries the
-- fingerprint `provider:model:dim` of the embedder that produced it, and
-- `embed_state` holds the fingerprint of the most recent query vector, so
-- retrieval only compares vectors from the same family (embed_fp.rs
-- `vector_family`). NULL `embed_fp` beside a vector means unknown origin:
-- excluded from vector search and re-embedded by the backfill.
ALTER TABLE chunks ADD COLUMN embed_fp TEXT;
CREATE INDEX IF NOT EXISTS ix_chunks_embed_fp ON chunks(embed_fp);
CREATE TABLE IF NOT EXISTS embed_state (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    active_fp TEXT,
    updated_at TEXT DEFAULT CURRENT_TIMESTAMP
);
-- One-shot backfill: every pre-R45.5 vector came from the HTTP embedder
-- (Ollama or an OpenAI-compatible endpoint), with the DB-configured model or
-- the default. An env-only ANCHOR_EMBED_MODEL override is invisible here; it
-- costs one re-embed after upgrade, never a mixed comparison.
UPDATE chunks
SET embed_fp = 'ollama:'
    || COALESCE((SELECT NULLIF(TRIM(value), '') FROM app_settings WHERE key = 'embed_model'), 'nomic-embed-text')
    || ':' || (length(embedding) / 4)
WHERE embedding IS NOT NULL;
