-- R45.8: structured fields from the deterministic record parser
-- (records.rs). `status` stays the review state (unverified/verified/...);
-- `record_status` is the decision's own state (accepted/superseded/...).
ALTER TABLE entities ADD COLUMN record_id TEXT NOT NULL DEFAULT '';
ALTER TABLE entities ADD COLUMN record_status TEXT NOT NULL DEFAULT '';
ALTER TABLE entities ADD COLUMN due TEXT NOT NULL DEFAULT '';
ALTER TABLE entities ADD COLUMN done INTEGER NOT NULL DEFAULT 0;
ALTER TABLE entities ADD COLUMN supersedes TEXT NOT NULL DEFAULT '';
ALTER TABLE entities ADD COLUMN superseded_by TEXT NOT NULL DEFAULT '';
CREATE INDEX IF NOT EXISTS ix_entities_owner ON entities(owner);
