//! R15.3: shared request-handler helpers — single DB-open, row-existence gate,
//! and `IN`-placeholder builder. Retrieval/answer/pipeline adopt where trivial;
//! each area's own task owns the rest.

use axum::{http::StatusCode, Json};
use serde_json::Value;

/// Open the data-dir DB with migrations, falling back to a plain open.
/// Replaces the `resolve_db_path + init_db + Connection::open` prelude
/// previously copied into every `spawn_blocking` handler.
pub fn open_data_db(data_dir: &str) -> rusqlite::Connection {
    let db_path = crate::db::resolve_db_path(data_dir);
    crate::db::init_db(&db_path).unwrap_or_else(|_| rusqlite::Connection::open(&db_path).unwrap())
}

/// `IN (?, ?, ...)` placeholders for `count` ids.
pub fn placeholders(count: usize) -> String {
    std::iter::repeat_n("?", count).collect::<Vec<_>>().join(",")
}

/// Whether a model/embed provider may receive sensitive/PII content.
/// Local endpoints are always trusted. Otherwise the provider must be listed in
/// `ANCHOR_TRUSTED_PROVIDERS` (comma-separated base-URL prefixes, e.g.
/// `https://llm-eu.corp.example.com,https://api.vendor.example.com`) or the
/// global `ANCHOR_CLOUD_TRUST=1` escape hatch must be set. Unset allowlist =
/// today’s local-only behavior. Prefix matching is boundary-safe: an entry
/// matches the base itself or a `/`-separated path under it, never a
/// lookalike host (`example.com.evil` does NOT match `example.com`).
pub fn provider_trusted(base_url: &str) -> bool {
    let base = base_url.trim().trim_end_matches('/');
    if base.starts_with("http://localhost") || base.starts_with("http://127.0.0.1") {
        return true;
    }
    if let Ok(list) = std::env::var("ANCHOR_TRUSTED_PROVIDERS") {
        for entry in list.split(',').map(|s| s.trim().trim_end_matches('/')).filter(|s| !s.is_empty()) {
            if base == entry || base.starts_with(&format!("{}/", entry)) {
                return true;
            }
        }
    }
    std::env::var("ANCHOR_CLOUD_TRUST").as_deref() == Ok("1")
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

#[cfg(test)]
mod tests {
    use super::*;

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
        let _allow = EnvGuard::remove("ANCHOR_TRUSTED_PROVIDERS");
        let _trust = EnvGuard::remove("ANCHOR_CLOUD_TRUST");
        // local always trusted, allowlist or not
        assert!(provider_trusted("http://localhost:11434"));
        assert!(provider_trusted("http://127.0.0.1:11434/v1"));
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
}
