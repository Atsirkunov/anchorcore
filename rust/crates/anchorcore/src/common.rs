//! R15.3: shared request-handler helpers — single DB-open, row-existence gate,
//! and `IN`-placeholder builder. Retrieval/answer/pipeline adopt where trivial;
//! each area's own task owns the rest.

use axum::{http::StatusCode, Json};
use serde_json::Value;

/// R48.1: open the data-dir DB (migrated at boot) with its PRAGMAs. Never
/// falls back to a bare `Connection::open`: that connection has no
/// `foreign_keys=ON` (source deletes skip their cascade) and no busy
/// timeout (writes fail on the first lock).
pub fn try_open_data_db(data_dir: &str) -> Result<rusqlite::Connection, String> {
    try_open_db_path(&crate::db::resolve_db_path(data_dir))
}

pub fn try_open_db_path(path: &std::path::Path) -> Result<rusqlite::Connection, String> {
    crate::db::init_db(path).map_err(|e| {
        tracing::error!(target: "anchorcore::db", "cannot open the database {}: {e}", path.display());
        format!("cannot open the database: {e}")
    })
}

/// [`try_open_data_db`] for callers without an error path: a failure panics
/// (handlers → 500 via the catch-panic layer, job bodies → `failed` via
/// `guard_job`) instead of continuing on a degraded connection.
pub fn open_data_db(data_dir: &str) -> rusqlite::Connection {
    open_db_path(&crate::db::resolve_db_path(data_dir))
}

pub fn open_db_path(path: &std::path::Path) -> rusqlite::Connection {
    try_open_db_path(path).unwrap_or_else(|e| panic!("{e}"))
}

pub type ApiError = (StatusCode, Json<Value>);

/// A database failure as a 500 `{"detail"}` (logged).
pub fn db_err(e: impl std::fmt::Display) -> ApiError {
    tracing::error!(target: "anchorcore::db", "{e}");
    (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"detail": format!("database error: {e}")})))
}

/// R43.11: the message of a caught panic payload (`panic!("…")` carries a
/// `&str` or a `String`; anything else is reported generically).
pub fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        return (*s).to_string();
    }
    payload.downcast_ref::<String>().cloned().unwrap_or_else(|| "non-string panic payload".to_string())
}

/// R48.11: a Sensitive-gate input that fails to read counts as Sensitive —
/// the gate fails closed. NULL keeps its meaning (no item/source: internal).
pub fn gate_label(r: &rusqlite::Row, idx: usize) -> String {
    match r.get::<_, Option<String>>(idx) {
        Ok(label) => label.unwrap_or_else(|| "internal".to_string()),
        Err(e) => {
            tracing::warn!(target: "anchorcore::gate", "label column unreadable, treated as sensitive: {e}");
            "sensitive".to_string()
        }
    }
}

/// R48.11: the per-document override column, failing closed like [`gate_label`].
pub fn gate_marked(r: &rusqlite::Row, idx: usize) -> bool {
    match r.get::<_, Option<i64>>(idx) {
        Ok(v) => v == Some(1),
        Err(e) => {
            tracing::warn!(target: "anchorcore::gate", "sensitive_override column unreadable, treated as set: {e}");
            true
        }
    }
}

/// Constant-time string equality for secrets (tokens, password hashes): the
/// time taken never depends on where the first difference is. Length is not
/// secret here (fixed-size tokens and hex digests).
pub fn ct_eq(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |diff, (x, y)| diff | (x ^ y)) == 0
}

/// R48.12: a write whose failure must not stop the caller — audit/event rows,
/// progress counters, tidy-ups, boot-time indexes — logged instead of ignored.
/// Anything the operation's result depends on uses `?` instead.
pub fn best_effort(what: &str, result: rusqlite::Result<usize>) {
    if let Err(e) = result {
        tracing::warn!(target: "anchorcore::db", "{what} failed: {e}");
    }
}

/// R48.11: rows that fail to decode are skipped but logged — never dropped
/// silently. For read paths where one bad row must not fail the whole list.
pub fn logged_rows<T>(rows: impl Iterator<Item = rusqlite::Result<T>>) -> impl Iterator<Item = T> {
    rows.filter_map(row_ok)
}

/// One row of [`logged_rows`]: `.filter_map(row_ok)`.
pub fn row_ok<T>(r: rusqlite::Result<T>) -> Option<T> {
    r.map_err(|e| tracing::warn!(target: "anchorcore::db", "row skipped: {e}")).ok()
}

/// R48.11: a read query that could not run, logged; the caller answers empty.
pub fn query_failed<T>(e: rusqlite::Error) -> Vec<T> {
    tracing::warn!(target: "anchorcore::db", "query failed: {e}");
    Vec::new()
}

/// `IN (?, ?, ...)` placeholders for `count` ids.
pub fn placeholders(count: usize) -> String {
    std::iter::repeat_n("?", count).collect::<Vec<_>>().join(",")
}

/// Whether a model/embed provider may receive Sensitive content.
/// Local endpoints are always trusted. Otherwise the provider must be listed in
/// `ANCHOR_TRUSTED_PROVIDERS` (comma-separated base-URL prefixes, e.g.
/// `https://llm-eu.corp.example.com,https://api.vendor.example.com`) or the
/// global `ANCHOR_CLOUD_TRUST=1` escape hatch must be set. Unset allowlist =
/// today’s local-only behavior. Prefix matching is boundary-safe: an entry
/// matches the base itself or a `/`-separated path under it, never a
/// lookalike host (`example.com.evil` does NOT match `example.com`).
/// Boundary-safe loopback check (F-06): a `starts_with("http://localhost")`
/// test trusts lookalikes (`http://localhost@evil`,
/// `http://localhost.evil.example`) while `reqwest` connects to the real
/// host — so parse the authority and compare the host exactly. Scope
/// matches the old check (`http` only); `::1` joins the loopback set.
pub fn provider_is_local(base_url: &str) -> bool {
    let base = base_url.trim().trim_end_matches('/');
    let Some(after_scheme) = base.strip_prefix("http://") else {
        return false;
    };
    let authority = after_scheme.split('/').next().unwrap_or("");
    // strip userinfo (`user@host`) — the host follows the last `@`
    let hostport = authority.rsplit('@').next().unwrap_or("");
    let host = if let Some(rest) = hostport.strip_prefix('[') {
        rest.split(']').next().unwrap_or("")
    } else {
        hostport.split(':').next().unwrap_or("")
    };
    matches!(host, "localhost" | "127.0.0.1" | "::1")
}

pub fn provider_trusted(base_url: &str) -> bool {
    provider_is_local(base_url) || provider_env_trusted(base_url)
}

/// R35.6: the admin overrides ("set by environment" in the Settings card):
/// the provider is listed in `ANCHOR_TRUSTED_PROVIDERS`, or the global
/// `ANCHOR_CLOUD_TRUST=1` is set. The person's per-destination switches
/// live in `sharing.rs`, whose `allowed` is the one rule every gate reads.
pub fn provider_env_trusted(base_url: &str) -> bool {
    let base = base_url.trim().trim_end_matches('/');
    if let Ok(list) = std::env::var("ANCHOR_TRUSTED_PROVIDERS") {
        for entry in list.split(',').map(|s| s.trim().trim_end_matches('/')).filter(|s| !s.is_empty()) {
            if base == entry || base.starts_with(&format!("{}/", entry)) {
                return true;
            }
        }
    }
    env_cloud_trust()
}

/// `ANCHOR_CLOUD_TRUST=1`: opens every destination, agents included (R35.6).
pub fn env_cloud_trust() -> bool {
    std::env::var("ANCHOR_CLOUD_TRUST").as_deref() == Ok("1")
}

/// R35.4: the `blocked` entry for content withheld because a person marked
/// its item Sensitive (`ingested_items.sensitive_override`), next to the
/// source labels.
pub const ITEM_SENSITIVE_MARK: &str = "item_sensitive";

/// R35.4: effective Sensitive — the per-document override or the source
/// label, derived at read time and never stored on the item (relabels, new
/// syncs and imports cannot drift).
pub fn effective_sensitive(source_label: &str, sensitive_override: bool) -> bool {
    sensitive_override || source_label == "sensitive"
}

/// SQL twin of `effective_sensitive` for gate predicates: `item` is the
/// `ingested_items` alias exposing `sensitive_override`, `label` a source
/// label expression. Parenthesised for AND/OR chains.
pub fn effective_sensitive_sql(item: &str, label: &str) -> String {
    format!("(COALESCE({item}.sensitive_override, 0) = 1 OR COALESCE({label}, '') = 'sensitive')")
}

/// R35.4: why content may not reach an untrusted cloud provider — its
/// effective Sensitive state (override or `sensitive` label). `None` = it
/// may flow. One rule for ask, search, `/decisions`, `/actions` and
/// `/media`; the label wins so the audit keeps naming the coarser cause.
pub fn cloud_gate_reason(label: &str, sensitive_override: bool) -> Option<String> {
    if !effective_sensitive(label, sensitive_override) {
        return None;
    }
    if label == "sensitive" {
        return Some(label.to_string());
    }
    Some(ITEM_SENSITIVE_MARK.to_string())
}

/// Require a row in `table` (`entities`/`sources`/`chunks`) or return 404.
/// `table` is always an internal constant, never user input.
pub fn require_row(
    conn: &rusqlite::Connection,
    table: &str,
    id: i64,
    not_found_detail: &str,
) -> Result<(), (StatusCode, Json<Value>)> {
    let sql = format!("SELECT 1 FROM {} WHERE id = ?1", table);
    let exists: bool = conn.query_row(&sql, [id], |_| Ok(())).is_ok();
    if exists {
        Ok(())
    } else {
        Err((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"detail": not_found_detail})),
        ))
    }
}

/// Serializes `ANCHOR_TRUSTED_PROVIDERS`/`ANCHOR_CLOUD_TRUST` mutation
/// between test modules (scheduler's backfill-gate tests share it).
#[cfg(test)]
pub(crate) static TRUST_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    fn pragma(conn: &rusqlite::Connection, name: &str) -> i64 {
        conn.query_row(&format!("PRAGMA {name}"), [], |r| r.get(0)).unwrap()
    }

    #[test]
    fn ct_eq_matches_plain_equality() {
        assert!(ct_eq("abc123", "abc123"));
        assert!(!ct_eq("abc123", "abc124"));
        assert!(!ct_eq("abc", "abc1"));
        assert!(ct_eq("", ""));
    }

    /// R48.11: gate columns fail closed on a read error; NULL keeps "internal" / unmarked.
    #[test]
    fn gate_columns_fail_closed() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let read = |sql: &str| conn.query_row(sql, [], |r| Ok((gate_label(r, 0), gate_marked(r, 1)))).unwrap();
        assert_eq!(read("SELECT 'public', 0"), ("public".to_string(), false));
        assert_eq!(read("SELECT NULL, NULL"), ("internal".to_string(), false));
        assert_eq!(read("SELECT 'internal', 1"), ("internal".to_string(), true));
        assert_eq!(read("SELECT 42, 'yes'"), ("sensitive".to_string(), true), "unreadable columns read as Sensitive");
    }

    /// R48.1: both the migrating open and the per-process fast path keep FK
    /// cascades and the busy wait; an unopenable path is an error, never a
    /// bare connection without them.
    #[test]
    fn data_db_opens_keep_foreign_keys_and_busy_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().to_string_lossy().to_string();
        for _ in 0..2 {
            let conn = try_open_data_db(&data_dir).unwrap();
            assert_eq!(pragma(&conn, "foreign_keys"), 1);
            assert_eq!(pragma(&conn, "busy_timeout"), 5000);
        }
        let blocker = dir.path().join("not-a-dir");
        std::fs::write(&blocker, b"x").unwrap();
        assert!(try_open_db_path(&blocker.join("anchorcore.db")).is_err());
    }

    struct EnvGuard {
        key: &'static str,
        prev: Option<String>,
    }

    impl EnvGuard {
        fn set(key: &'static str, val: &str) -> Self {
            let prev = std::env::var(key).ok();
            std::env::set_var(key, val);
            Self { key, prev }
        }

        fn remove(key: &'static str) -> Self {
            let prev = std::env::var(key).ok();
            std::env::remove_var(key);
            Self { key, prev }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            if let Some(v) = &self.prev {
                std::env::set_var(self.key, v);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }

    // One test (not three) so parallel runners can't interleave env mutations.
    #[test]
    fn trusted_providers() {
        let _lock = TRUST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _allow = EnvGuard::remove("ANCHOR_TRUSTED_PROVIDERS");
        let _trust = EnvGuard::remove("ANCHOR_CLOUD_TRUST");
        // local always trusted, allowlist or not
        assert!(provider_trusted("http://localhost:11434"));
        assert!(provider_trusted("http://127.0.0.1:11434/v1"));
        // F-06: localhost lookalikes are NOT local (host-boundary check)
        assert!(!provider_is_local("http://localhost@evil.example/v1"));
        assert!(!provider_is_local("http://localhost.evil.example/v1"));
        assert!(!provider_is_local("http://127.0.0.1@evil.example/v1"));
        assert!(!provider_is_local("http://127.0.0.1.evil.example/v1"));
        assert!(!provider_is_local("http://localhost:11434@evil.example/v1"));
        assert!(!provider_trusted("http://localhost@evil.example/v1"));
        assert!(!provider_trusted("http://localhost.evil.example/v1"));
        // userinfo on a real loopback host stays trusted; ::1 joins the set
        assert!(provider_is_local("http://user@localhost:11434/v1"));
        assert!(provider_is_local("http://[::1]:11434/v1"));
        assert!(provider_is_local("http://localhost"));
        // default deny for unlisted remotes
        assert!(!provider_trusted("https://api.openai.com/v1"));

        let _allow = EnvGuard::set("ANCHOR_TRUSTED_PROVIDERS", "https://llm-eu.corp.example.com, https://api.vendor.example.com/");
        assert!(provider_trusted("https://llm-eu.corp.example.com"));
        assert!(provider_trusted("https://llm-eu.corp.example.com/v1/chat"));
        assert!(provider_trusted("https://api.vendor.example.com"));
        assert!(!provider_trusted("https://llm-eu.corp.example.com.evil.com"));
        assert!(!provider_trusted("https://other.example.com"));

        // global hatch still works on top of the allowlist
        let _trust = EnvGuard::set("ANCHOR_CLOUD_TRUST", "1");
        assert!(provider_trusted("https://other.example.com"));
    }

    #[test]
    fn cloud_gate_reason_label_then_item_mark() {
        assert_eq!(cloud_gate_reason("sensitive", false).as_deref(), Some("sensitive"));
        assert_eq!(cloud_gate_reason("sensitive", true).as_deref(), Some("sensitive"), "label names the cause first");
        assert_eq!(cloud_gate_reason("internal", true).as_deref(), Some(ITEM_SENSITIVE_MARK));
        assert_eq!(cloud_gate_reason("public", true).as_deref(), Some(ITEM_SENSITIVE_MARK));
        assert_eq!(cloud_gate_reason("internal", false), None);
        assert_eq!(cloud_gate_reason("public", false), None);
        assert!(effective_sensitive("internal", true), "override escalates whatever the label");
        assert!(!effective_sensitive("public", false), "unmarked public flows");
    }
}
