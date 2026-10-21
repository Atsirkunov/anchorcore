-- R45.13: vocabulary without proposals. A person marks a file as a
-- glossary (`PATCH /items/:id {"is_glossary": true}`); its rows join the
-- `vocabulary` table as origin 'glossary', typed-in entries as 'manual'.
-- The Rust hook (`vocab::prune_to_trusted`, db.rs) then drops every LLM
-- row (proposed, rejected, and corroborated alike — an R40.8 approval left
-- no trace that tells it from corroboration) and every rule row that fails
-- the initialism check. Rule rows that pass stay (re-sync re-derives the
-- rest; the check is pure Rust, so it cannot live in this SQL).
ALTER TABLE ingested_items ADD COLUMN is_glossary INTEGER NOT NULL DEFAULT 0;
CREATE INDEX IF NOT EXISTS ix_vocabulary_origin ON vocabulary(origin, item_id);
