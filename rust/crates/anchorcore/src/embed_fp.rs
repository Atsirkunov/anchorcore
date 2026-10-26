//! R45.5: vector provenance — fingerprints, families, and the active query
//! fingerprint. `Connection`-level only (no settings), so the retrieval eval
//! includes it next to `retrieval.rs`.
//!
//! Every stored chunk vector carries `chunks.embed_fp` = `provider:model:dim`
//! (exact provenance), and `embed_state.active_fp` holds the fingerprint of
//! the latest query vector. Comparability is decided by *family*, not by the
//! exact string: two fingerprints compare only when their families and dims
//! match. Unknown models are their own family (`provider:model`), so any model
//! or provider change re-embeds; the explicit table below lists the models
//! whose weights are proven identical across providers.

use rusqlite::Connection;

/// Provider tag of the HTTP embedder — Ollama or any OpenAI-compatible
/// endpoint (the model name, not the URL, identifies the weights, so a
/// remote Ollama serving the same model stays comparable). R45.4's built-in
/// engine stamps `builtin`.
pub const PROVIDER_HTTP: &str = "ollama";

const NOMIC_V15: &str = "nomic-embed-text-v1.5";

/// `(provider, normalized model, family)`: the probe in
/// `docs/archive/handover-2026-10-21-llm-modes.md` §3 measured cosine 1.00000 between
/// candle nomic v1.5 and Ollama `nomic-embed-text` (with the R43.6 prefixes),
/// so those share a family and switching between them needs no re-embed.
/// A remote OpenAI-compatible "nomic" is NOT listed — unproven, re-embed.
const SHARED_FAMILIES: &[(&str, &str, &str)] = &[
    ("ollama", "nomic-embed-text", NOMIC_V15),
    ("ollama", "nomic-embed-text:v1.5", NOMIC_V15),
    ("builtin", "nomic-embed-text-v1.5", NOMIC_V15),
    ("builtin", "nomic-ai/nomic-embed-text-v1.5", NOMIC_V15),
];

fn normalize_model(model: &str) -> String {
    let m = model.trim().to_lowercase();
    m.strip_suffix(":latest").map(str::to_string).unwrap_or(m)
}

/// Exact provenance string stored per vector.
pub fn fingerprint(provider: &str, model: &str, dim: usize) -> String {
    format!("{provider}:{}:{dim}", normalize_model(model))
}

/// Family of a provider/model pair (dim-free; dim is checked separately).
pub fn model_family(provider: &str, model: &str) -> String {
    let model = normalize_model(model);
    SHARED_FAMILIES
        .iter()
        .find(|(p, m, _)| *p == provider && *m == model)
        .map(|(_, _, family)| family.to_string())
        .unwrap_or_else(|| format!("{provider}:{model}"))
}

/// Parse `provider:model:dim` (the model may itself contain `:`) into its
/// `(family, dim)`. `None` for malformed strings — never comparable.
pub fn vector_family(fp: &str) -> Option<(String, usize)> {
    let (provider, rest) = fp.split_once(':')?;
    let (model, dim) = rest.rsplit_once(':')?;
    let dim = dim.parse().ok()?;
    if provider.is_empty() || model.is_empty() {
        return None;
    }
    Some((model_family(provider, model), dim))
}

/// Whether vectors stamped `a` and `b` may be compared by cosine.
pub fn same_vector_family(a: &str, b: &str) -> bool {
    match (vector_family(a), vector_family(b)) {
        (Some(x), Some(y)) => x == y,
        _ => false,
    }
}

/// Fingerprint of the latest query vector; `None` before the first query
/// (and in hash-embedding tests), which means "nothing to filter against".
pub fn active_fingerprint(conn: &Connection) -> Option<String> {
    conn.query_row("SELECT active_fp FROM embed_state WHERE id = 1", [], |r| r.get(0))
        .ok()
        .flatten()
}

/// Record a query vector's fingerprint. Returns true when there was no
/// previous fingerprint or its family differs (re-embed needed); a family
/// change also records a `system_events` row so the switch is visible.
pub fn record_active_fingerprint(conn: &Connection, fp: &str) -> Result<bool, String> {
    let prev = active_fingerprint(conn);
    if prev.as_deref() == Some(fp) {
        return Ok(false);
    }
    conn.execute(
        "INSERT INTO embed_state (id, active_fp, updated_at) VALUES (1, ?1, CURRENT_TIMESTAMP) \
         ON CONFLICT(id) DO UPDATE SET active_fp = excluded.active_fp, updated_at = excluded.updated_at",
        [fp],
    )
    .map_err(|e| e.to_string())?;
    let Some(prev) = prev else { return Ok(true) };
    if same_vector_family(&prev, fp) {
        return Ok(false);
    }
    let msg = format!("embedding model changed ({prev} -> {fp}); older vectors are keyword-only until re-embedded");
    tracing::warn!("{}", msg);
    let _ = conn.execute(
        "INSERT INTO system_events (component, level, message, detail) VALUES ('embedder','warning',?1,'')",
        [msg],
    );
    Ok(true)
}

/// Distinct stored fingerprints, split by comparability with `family`
/// (dim-free family of the configured embedder): `(fresh, stale)`.
pub fn partition_stored_fingerprints(conn: &Connection, family: &str) -> (Vec<String>, Vec<String>) {
    stored_fingerprints(conn)
        .into_iter()
        .partition(|fp| vector_family(fp).is_some_and(|(f, _)| f == family))
}

/// Stored fingerprints comparable with the active query fingerprint.
/// `None` = no active fingerprint (compare everything, pre-R45.5 behavior);
/// `Some(empty)` = nothing comparable (the vector legs return no hits).
pub fn comparable_fingerprints(conn: &Connection) -> Option<Vec<String>> {
    let active = active_fingerprint(conn)?;
    Some(stored_fingerprints(conn).into_iter().filter(|fp| same_vector_family(fp, &active)).collect())
}

/// Status view: the active query fingerprint + how many stored vectors the
/// vector legs currently ignore (unstamped or another family). Zero before
/// the first query, when there is nothing to compare against yet.
pub fn stale_vector_status(conn: &Connection) -> (Option<String>, i64) {
    let active = active_fingerprint(conn);
    let Some(comparable) = comparable_fingerprints(conn) else {
        return (active, 0);
    };
    let total: i64 = conn
        .query_row("SELECT COUNT(*) FROM chunks WHERE embedding IS NOT NULL", [], |r| r.get(0))
        .unwrap_or(0);
    let fresh: i64 = comparable
        .iter()
        .map(|fp| {
            conn.query_row("SELECT COUNT(*) FROM chunks WHERE embedding IS NOT NULL AND embed_fp = ?1", [fp], |r| r.get::<_, i64>(0))
                .unwrap_or(0)
        })
        .sum();
    (active, total - fresh)
}

/// Distinct stamped fingerprints (index-backed, a handful of rows).
fn stored_fingerprints(conn: &Connection) -> Vec<String> {
    conn.prepare("SELECT DISTINCT embed_fp FROM chunks WHERE embed_fp IS NOT NULL")
        .and_then(|mut s| s.query_map([], |r| r.get(0))?.collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vector_fingerprint_families() {
        // Exact provenance string, family-level comparability.
        assert_eq!(fingerprint("ollama", "Nomic-Embed-Text:latest", 768), "ollama:nomic-embed-text:768");
        assert_eq!(vector_family("ollama:nomic-embed-text:768"), Some((NOMIC_V15.to_string(), 768)));
        // model names may contain ':' — dim is after the last one
        assert_eq!(vector_family("ollama:qwen3-embedding:4b:2560"), Some(("ollama:qwen3-embedding:4b".to_string(), 2560)));
        assert!(same_vector_family("ollama:nomic-embed-text:768", "builtin:nomic-embed-text-v1.5:768"));
        assert!(same_vector_family("ollama:nomic-embed-text:768", "ollama:nomic-embed-text:v1.5:768"));
        assert!(!same_vector_family("ollama:nomic-embed-text:768", "ollama:nomic-embed-text:256"));
        assert!(!same_vector_family("ollama:nomic-embed-text:768", "ollama:mxbai-embed-large:768"));
        // unproven providers never join a shared family
        assert!(!same_vector_family("ollama:nomic-embed-text:768", "cloud:nomic-embed-text:768"));
        for bad in ["", "ollama", "ollama:768", ":m:768", "ollama:m:x"] {
            assert_eq!(vector_family(bad), None, "{bad}");
            assert!(!same_vector_family(bad, bad), "{bad}");
        }
    }

    #[test]
    fn stale_vector_status_counts_incomparable_vectors() {
        let dir = tempfile::tempdir().unwrap();
        let conn = crate::db::init_db(&dir.path().join("fp.db")).unwrap();
        conn.execute("INSERT INTO sources (name, connector) VALUES ('s','folder')", []).unwrap();
        conn.execute("INSERT INTO ingested_items (source_id, content_hash) VALUES (1,'h')", []).unwrap();
        for fp in [Some("ollama:nomic-embed-text:768"), Some("ollama:mxbai-embed-large:768"), None] {
            conn.execute(
                "INSERT INTO chunks (item_id, content, embedding, embed_fp) VALUES (1, 'c', zeroblob(8), ?1)",
                [fp],
            )
            .unwrap();
        }
        assert_eq!(stale_vector_status(&conn), (None, 0), "no query yet: nothing to compare against");
        record_active_fingerprint(&conn, "builtin:nomic-embed-text-v1.5:768").unwrap();
        let (active, stale) = stale_vector_status(&conn);
        assert_eq!(active.as_deref(), Some("builtin:nomic-embed-text-v1.5:768"));
        assert_eq!(stale, 2, "other family + unstamped are stale; ollama nomic shares the family");
    }
}
