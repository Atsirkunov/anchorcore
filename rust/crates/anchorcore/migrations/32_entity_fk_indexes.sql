-- R41.8: every child of `entities` (ON DELETE CASCADE) lacked an index, so
-- with foreign keys on each deleted entity scanned the whole `chunks` table.
-- The R45.10 cleanup (migration 27) deletes thousands of entities at once and
-- took ~5 min on a 5.5 GB library; reclassify deletes entities per item too.
-- The migration 27 hook runs this file first, so pre-v27 DBs get the indexes
-- before their cleanup. Few chunks carry an entity, hence the partial index.
CREATE INDEX IF NOT EXISTS ix_chunks_entity_id ON chunks(entity_id) WHERE entity_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS ix_relationships_from_entity_id ON relationships(from_entity_id);
CREATE INDEX IF NOT EXISTS ix_relationships_to_entity_id ON relationships(to_entity_id);
CREATE INDEX IF NOT EXISTS ix_merge_actions_entity_a_id ON merge_actions(entity_a_id);
CREATE INDEX IF NOT EXISTS ix_merge_actions_entity_b_id ON merge_actions(entity_b_id);
CREATE INDEX IF NOT EXISTS ix_disputes_entity_id ON disputes(entity_id);
CREATE INDEX IF NOT EXISTS ix_entities_item_id ON entities(item_id);
