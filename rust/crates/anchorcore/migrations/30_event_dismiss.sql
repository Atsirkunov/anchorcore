-- R47.2: dismissable system events. A person clears the System tab's
-- errors & warnings panel by stamping `system_events.dismissed_at`; rows are
-- never deleted here — the table is also the audit trail (MCP calls, gate
-- refusals) and feeds per-item flags (`pdf split-word repair`). The column
-- is added by the Rust hook (`add_event_dismissed_at`, db.rs) behind a
-- column-existence guard: migration 13 rebuilds the table, so whether the
-- column already exists depends on the DB's history.
CREATE INDEX IF NOT EXISTS ix_system_events_created ON system_events(created_at);
