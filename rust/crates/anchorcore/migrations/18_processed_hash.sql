-- F-03: per-item completion marker. `content_hash` says what is STORED;
-- `processed_hash` says what was FULLY processed (classify + every finalize
-- phase). A cancel/failure between upsert and finalize used to leave the
-- item marked current forever; now the next sync re-processes it.
-- One-shot backfill: items processed before this version carry no marker,
-- so adopt their stored hash (a zero-cost upgrade — no corpus-wide
-- reclassify, no chunk-uid churn). Items interrupted BEFORE this upgrade
-- stay silently partial and need one force re-sync to heal.
ALTER TABLE ingested_items ADD COLUMN processed_hash TEXT;
UPDATE ingested_items SET processed_hash = content_hash WHERE processed_hash IS NULL;
