//! Redacted bug-report bundle — the "it's wrong" flow (R47.0).
//!
//! Retention: [`record_ask`]/[`record_search`] (called from the `routes.rs`
//! handlers, so UI, REST-agent and MCP-over-HTTP traffic is all covered)
//! keep metadata-only entries for the last [`REPORT_RETAIN`] queries in a
//! process-global ring. Black-box tests spawn one process per test file, so
//! the ring never leaks between tests; unit tests must not assert on it.
//!
//! Generation: `GET /system/report` maps one entry to a [`Report`] whose
//! serialized shape contains no document text, titles, paths, settings or
//! secrets by construction — the structs lack those fields. Titles and
//! source names map to stable pseudonyms plus hashes for cross-report
//! correlation. The query text itself is kept: it is the reporter's own
//! input and the report is useless for repro without it.

use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, OnceLock};

use axum::{extract::{Query, State}, http::StatusCode, Json};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Queries retained for reporting (metadata only, ~10 KB each worst case).
pub const REPORT_RETAIN: usize = 20;
/// Serialized [`Report`] shape version.
pub const REPORT_VERSION: u32 = 1;

// ---------------------------------------------------------------------------
// Retention (in-memory; real ids/titles never leave this module unmapped)
// ---------------------------------------------------------------------------

/// One ranked hit, metadata only — deliberately no content/summary/snippet.
#[derive(Debug, Clone)]
pub struct LoggedHit {
    pub item_id: Option<i64>,
    pub section_id: Option<i64>,
    pub kind: String,
    pub doc_type: String,
    pub env: String,
    pub score: f64,
    pub decay: f64,
    pub boosted: bool,
    pub provenance: Vec<crate::retrieval::LegMatch>,
    pub pierced: bool,
    pub item_title: String,
    pub path: String,
}

impl LoggedHit {
    pub fn from_search_hit(h: &crate::answer::SearchHit) -> Self {
        Self {
            item_id: h.item_id,
            section_id: h.section_id,
            kind: h.kind.clone(),
            doc_type: h.doc_type.clone(),
            env: h.env.clone(),
            score: h.score,
            decay: h.decay,
            boosted: h.boosted,
            provenance: h.provenance.clone(),
            pierced: h.rich_match,
            item_title: h.item_title.clone(),
            path: h.path.clone(),
        }
    }

    pub fn from_citation(c: &crate::answer::Citation) -> Self {
        Self {
            item_id: c.item_id,
            section_id: c.section_id,
            kind: c.kind.clone(),
            doc_type: c.doc_type.clone(),
            env: c.env.clone(),
            score: c.score,
            decay: c.decay,
            boosted: c.boosted,
            provenance: c.provenance.clone(),
            pierced: c.rich_match,
            item_title: c.item_title.clone(),
            path: c.path.clone(),
        }
    }
}

/// One executed query: request text plus the ranked response, metadata only.
#[derive(Debug, Clone)]
pub struct LoggedQuery {
    pub asked_at: String,
    pub kind: &'static str,
    pub scope: String,
    pub query: String,
    pub retrieval: String,
    pub mode: Option<String>,
    pub blocked: usize,
    pub refusal: Option<String>,
    pub hits: Vec<LoggedHit>,
}

static RING: OnceLock<Mutex<VecDeque<LoggedQuery>>> = OnceLock::new();

fn ring() -> &'static Mutex<VecDeque<LoggedQuery>> {
    RING.get_or_init(|| Mutex::new(VecDeque::new()))
}

/// Push one entry, evicting the oldest past [`REPORT_RETAIN`]. Best-effort:
/// a poisoned lock must never break serving.
pub fn record(entry: LoggedQuery) {
    if let Ok(mut ring) = ring().lock() {
        ring.push_back(entry);
        while ring.len() > REPORT_RETAIN {
            ring.pop_front();
        }
    }
}

/// `index` back from the newest (`0` = last query). `None` when empty or out
/// of range (and on a poisoned lock — same best-effort rule).
pub fn get(index_from_last: usize) -> Option<LoggedQuery> {
    ring().lock().ok()?.iter().rev().nth(index_from_last).cloned()
}

/// Scope label for a retained entry: project, public-only, or everything.
pub fn scope_label(project_id: Option<i64>, public_only: bool) -> String {
    if public_only {
        "public".to_string()
    } else if let Some(id) = project_id {
        format!("project:{id}")
    } else {
        "all".to_string()
    }
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Retention hook for `POST /qa` (and `/qa/public`) — call from the handler
/// with the already-built response so no return path is missed.
pub fn record_ask(question: &str, scope: String, resp: &crate::answer::AskResponse) {
    record(LoggedQuery {
        asked_at: now_rfc3339(),
        kind: "ask",
        scope,
        query: question.to_string(),
        retrieval: resp.retrieval.clone(),
        mode: Some(resp.mode.clone()),
        blocked: 0,
        refusal: resp.refusal.clone(),
        hits: resp.citations.iter().map(LoggedHit::from_citation).collect(),
    });
}

/// Retention hook for `POST /qa/search` — same handler-level contract.
pub fn record_search(query: &str, scope: String, resp: &crate::answer::SearchResponse) {
    record(LoggedQuery {
        asked_at: now_rfc3339(),
        kind: "search",
        scope,
        query: query.to_string(),
        retrieval: resp.retrieval.clone(),
        mode: None,
        blocked: resp.blocked,
        refusal: resp.refusal.clone(),
        hits: resp.hits.iter().map(LoggedHit::from_search_hit).collect(),
    });
}

// ---------------------------------------------------------------------------
// Redaction mappers (pure — unit-tested)
// ---------------------------------------------------------------------------

/// Stable `item-N` pseudonyms in order of first appearance (`None` item ids
/// share one `item-?` bucket — they carry no identity to protect).
pub fn item_pseudonyms(items_in_order: &[Option<i64>]) -> HashMap<Option<i64>, String> {
    let mut map = HashMap::new();
    let mut next = 1usize;
    for item in items_in_order {
        if !map.contains_key(item) {
            let label = item.map_or_else(|| "item-?".to_string(), |_| {
                let label = format!("item-{next}");
                next += 1;
                label
            });
            map.insert(*item, label);
        }
    }
    map
}

/// Correlation hash for a title: sha256 hex truncated to 16 chars. Equal
/// titles hash equal across reports; nothing readable leaks.
pub fn title_hash(title: &str) -> String {
    crate::hashing::content_hash(title).chars().take(16).collect()
}

/// Lowercased file extension from a title or path (`"pdf"`, `"go"`, `""`
/// when none) — file-kind signal for tier analysis without the name.
pub fn ext_of(name_or_path: &str) -> String {
    let base = name_or_path.rsplit(['/', '\\']).next().unwrap_or(name_or_path);
    match base.rsplit('.').next() {
        Some(ext) if ext.len() < base.len() => ext.to_ascii_lowercase(),
        _ => String::new(),
    }
}

// ---------------------------------------------------------------------------
// Report shape (serialized — every field is safe to share)
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct ReportQuery {
    pub text: String,
    pub kind: String,
    pub scope: String,
    pub asked_at: String,
    pub retrieval: String,
    pub mode: Option<String>,
    pub blocked: usize,
    pub refusal: Option<String>,
    pub hits_returned: usize,
}

#[derive(Debug, Serialize)]
pub struct ReportHit {
    pub item_ref: String,
    pub title_hash: String,
    pub title_len: usize,
    pub ext: String,
    pub kind: String,
    pub doc_type: String,
    pub env: String,
    pub score: f64,
    pub decay: f64,
    pub boosted: bool,
    pub pierced: bool,
    pub provenance: Vec<crate::retrieval::LegMatch>,
}

#[derive(Debug, Serialize)]
pub struct ReportSource {
    pub source_ref: String,
    pub kind: String,
    pub label: String,
    pub items: i64,
    pub error_count: i64,
}

#[derive(Debug, Serialize)]
pub struct ReportIndex {
    pub sources: Vec<ReportSource>,
    pub items: i64,
    pub chunks: i64,
    pub pending_embeddings: i64,
    pub stale_vectors: i64,
}

#[derive(Debug, Serialize)]
pub struct ReportEnv {
    pub version: String,
    pub llm_mode: String,
    pub embed_family: Option<String>,
    pub ollama: String,
}

#[derive(Debug, Serialize)]
pub struct Report {
    pub report_version: u32,
    pub generated_at: String,
    pub query: ReportQuery,
    pub hits: Vec<ReportHit>,
    pub index: ReportIndex,
    pub environment: ReportEnv,
}

fn map_query(entry: &LoggedQuery) -> ReportQuery {
    ReportQuery {
        text: entry.query.clone(),
        kind: entry.kind.to_string(),
        scope: entry.scope.clone(),
        asked_at: entry.asked_at.clone(),
        retrieval: entry.retrieval.clone(),
        mode: entry.mode.clone(),
        blocked: entry.blocked,
        refusal: entry.refusal.clone(),
        hits_returned: entry.hits.len(),
    }
}

fn map_hits(entry: &LoggedQuery) -> Vec<ReportHit> {
    let order: Vec<Option<i64>> = entry.hits.iter().map(|h| h.item_id).collect();
    let refs = item_pseudonyms(&order);
    entry
        .hits
        .iter()
        .map(|h| {
            let ext = ext_of(&h.item_title);
            let ext = if ext.is_empty() { ext_of(&h.path) } else { ext };
            ReportHit {
                item_ref: refs.get(&h.item_id).cloned().unwrap_or_else(|| "item-?".to_string()),
                title_hash: title_hash(&h.item_title),
                title_len: h.item_title.chars().count(),
                ext,
                kind: h.kind.clone(),
                doc_type: h.doc_type.clone(),
                env: h.env.clone(),
                score: h.score,
                decay: h.decay,
                boosted: h.boosted,
                pierced: h.pierced,
                provenance: h.provenance.clone(),
            }
        })
        .collect()
}

fn source_rows(conn: &Connection) -> Vec<ReportSource> {
    let mut stmt = match conn.prepare(
        "SELECT s.connector, s.label, s.error_count, COUNT(i.id) FROM sources s \
         LEFT JOIN ingested_items i ON i.source_id = s.id GROUP BY s.id ORDER BY s.id",
    ) {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    let rows: Vec<(String, String, i64, i64)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .map(|m| m.filter_map(|r| r.ok()).collect())
        .unwrap_or_default();
    rows.into_iter()
        .enumerate()
        .map(|(n, (kind, label, error_count, items))| ReportSource {
            source_ref: format!("source-{}", n + 1),
            kind,
            label,
            items,
            error_count,
        })
        .collect()
}

fn index_snapshot(conn: &Connection) -> ReportIndex {
    let sources = source_rows(conn);
    let items: i64 =
        conn.query_row("SELECT COUNT(*) FROM ingested_items", [], |r| r.get(0)).unwrap_or(0);
    let chunks: i64 = conn.query_row("SELECT COUNT(*) FROM chunks", [], |r| r.get(0)).unwrap_or(0);
    let pending: i64 = conn
        .query_row("SELECT COUNT(*) FROM chunks WHERE embedding IS NULL", [], |r| r.get(0))
        .unwrap_or(0);
    let (_, stale_vectors) = crate::embed_fp::stale_vector_status(conn);
    ReportIndex { sources, items, chunks, pending_embeddings: pending, stale_vectors }
}

/// Assemble one redacted report. `llm_mode`/`ollama` arrive pre-gathered by
/// the (async) handler; everything else reads from `conn`.
pub fn generate(conn: &Connection, entry: &LoggedQuery, llm_mode: &str, ollama: &str) -> Report {
    let (active_fp, _) = crate::embed_fp::stale_vector_status(conn);
    Report {
        report_version: REPORT_VERSION,
        generated_at: now_rfc3339(),
        query: map_query(entry),
        hits: map_hits(entry),
        index: index_snapshot(conn),
        environment: ReportEnv {
            version: env!("CARGO_PKG_VERSION").to_string(),
            llm_mode: llm_mode.to_string(),
            embed_family: active_fp,
            ollama: ollama.to_string(),
        },
    }
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct ReportParams {
    /// 0 = last query (default), 1 = previous, …
    pub index: Option<usize>,
}

fn not_found(detail: String) -> (StatusCode, Json<Value>) {
    (StatusCode::NOT_FOUND, Json(json!({"detail": detail})))
}

pub async fn report_handler(
    State(state): State<crate::health::AppState>,
    Query(params): Query<ReportParams>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let index = params.index.unwrap_or(0);
    let Some(entry) = get(index) else {
        let detail = ring()
            .lock()
            .ok()
            .filter(|ring| !ring.is_empty())
            .map(|ring| format!("No query at index {index} ({} recorded)", ring.len()))
            .unwrap_or_else(|| "No queries recorded yet".to_string());
        return Err(not_found(detail));
    };
    let readiness = crate::ollama::model_readiness(&state.settings).await;
    let ollama = if readiness.reachable { "ok" } else { "offline" }.to_string();
    let llm_mode = crate::settings::llm_mode(&state.settings).to_string();
    let data_dir = state.data_dir.clone();
    let report = tokio::task::spawn_blocking(move || {
        let db_path = crate::db::resolve_db_path(&data_dir);
        let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| Connection::open(&db_path).unwrap());
        generate(&conn, &entry, &llm_mode, &ollama)
    })
    .await
    .map_err(|e| {
        (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"detail": format!("report failed: {e}")})))
    })?;
    serde_json::to_value(report)
        .map(Json)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"detail": format!("report failed: {e}")}))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pseudonyms_stable_by_first_appearance() {
        let map = item_pseudonyms(&[Some(9), Some(3), Some(9), None, Some(3), None]);
        assert_eq!(map[&Some(9)], "item-1");
        assert_eq!(map[&Some(3)], "item-2");
        assert_eq!(map[&None], "item-?");
    }

    #[test]
    fn title_hash_deterministic_and_truncated() {
        let a = title_hash("Direct Share Registration Product.pdf");
        assert_eq!(a.len(), 16);
        assert_eq!(a, title_hash("Direct Share Registration Product.pdf"));
        assert_ne!(a, title_hash("Something else entirely.md"));
    }

    #[test]
    fn ext_lowercases_last_segment() {
        assert_eq!(ext_of("Report.PDF"), "pdf");
        assert_eq!(ext_of("a/b/service.go"), "go");
        assert_eq!(ext_of("noext"), "");
        assert_eq!(ext_of(""), "");
    }

    #[test]
    fn scope_labels_cover_routes() {
        assert_eq!(scope_label(None, false), "all");
        assert_eq!(scope_label(None, true), "public");
        assert_eq!(scope_label(Some(7), false), "project:7");
    }
}
