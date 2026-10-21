//! R45.9: decision log + action tracker over the R45.8 record columns.
//!
//! REST is the contract (`GET /decisions`, `GET /actions`); the MCP
//! `decisions`/`actions` tools wrap these exact endpoints — no feature is
//! MCP-only. Scope and gating mirror `/qa/search` for the same caller:
//! `project_id` + `public_only` resolve through `answer::project_scope`,
//! stale (and, per `ANCHOR_QA_EXCLUDE_DISPUTED`, disputed) rows drop via
//! `retrieval::status_ok`, and an untrusted answer provider withholds
//! sensitive/pii-source rows (`blocked`, `refusal: "gated_provider"`).

use axum::extract::{Query, State};
use axum::Json;
use rusqlite::Connection;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

use crate::health::AppState;

const DEFAULT_LIMIT: usize = 100;
const MAX_LIMIT: usize = 500;

#[derive(Debug, Default, Clone, Deserialize)]
pub struct RecordQuery {
    /// Case-insensitive substring over the summary/task.
    pub q: Option<String>,
    /// Decisions: effective status (`accepted`, `superseded`, …) or
    /// `current` (neither superseded nor rejected).
    pub status: Option<String>,
    /// Actions: case-insensitive owner name.
    pub owner: Option<String>,
    /// Actions: `open` (default), `done` or `all`.
    pub state: Option<String>,
    /// Actions: only open items whose ISO `due` date is before today.
    pub overdue: Option<bool>,
    /// Comma list of source ids (intersected with the project scope).
    pub source_ids: Option<String>,
    pub project_id: Option<i64>,
    pub public_only: Option<bool>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

#[derive(Debug, Clone)]
struct Row {
    id: i64,
    summary: String,
    review_status: String,
    owner: String,
    record_id: String,
    record_status: String,
    due: String,
    done: bool,
    supersedes: Vec<String>,
    superseded_by: String,
    item_id: i64,
    path: String,
    title: String,
    source_id: i64,
    label: String,
    chunk_id: Option<i64>,
    created_at: String,
}

fn row_from_sql(r: &rusqlite::Row) -> rusqlite::Result<Row> {
    let supersedes: String = r.get(8)?;
    Ok(Row {
        id: r.get(0)?,
        summary: r.get(1)?,
        review_status: r.get(2)?,
        owner: r.get(3)?,
        record_id: r.get(4)?,
        record_status: r.get(5)?,
        due: r.get(6)?,
        done: r.get::<_, i64>(7)? != 0,
        supersedes: supersedes.split(',').map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect(),
        superseded_by: r.get(9)?,
        item_id: r.get(10)?,
        path: r.get::<_, Option<String>>(11)?.unwrap_or_default(),
        title: r.get::<_, Option<String>>(12)?.unwrap_or_default(),
        source_id: r.get(13)?,
        label: r.get::<_, Option<String>>(14)?.unwrap_or_default(),
        chunk_id: r.get(15)?,
        created_at: r.get::<_, Option<String>>(16)?.unwrap_or_default(),
    })
}

const BASE_SQL: &str = "SELECT e.id, e.summary, e.status, e.owner, e.record_id, e.record_status, e.due, e.done, \
     e.supersedes, e.superseded_by, e.item_id, i.external_id, i.title, i.source_id, s.label, \
     (SELECT c.id FROM chunks c WHERE c.entity_id = e.id ORDER BY c.id LIMIT 1), e.created_at \
     FROM entities e JOIN ingested_items i ON i.id = e.item_id JOIN sources s ON s.id = i.source_id \
     WHERE e.kind = ?";

/// Requested source ids ∩ the search scope (`None` = everything).
fn resolve_scope(conn: &Connection, q: &RecordQuery) -> Option<HashSet<i64>> {
    let scope = crate::answer::project_scope(conn, q.project_id, q.public_only.unwrap_or(false));
    let requested: Option<HashSet<i64>> = q
        .source_ids
        .as_deref()
        .map(|s| s.split(',').filter_map(|p| p.trim().parse().ok()).collect())
        .filter(|s: &HashSet<i64>| !s.is_empty());
    match (scope, requested) {
        (Some(a), Some(b)) => Some(a.intersection(&b).copied().collect()),
        (a, b) => a.or(b),
    }
}

fn push_filters(sql: &mut String, params: &mut Vec<rusqlite::types::Value>, kind: &str, q: &RecordQuery) {
    use rusqlite::types::Value as V;
    params.push(V::Text(kind.to_string()));
    if let Some(text) = q.q.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
        let esc = text.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
        sql.push_str(" AND e.summary LIKE ? ESCAPE '\\'");
        params.push(V::Text(format!("%{esc}%")));
    }
    if kind != "action" {
        return;
    }
    if let Some(owner) = q.owner.as_deref().map(str::trim).filter(|o| !o.is_empty()) {
        sql.push_str(" AND lower(e.owner) = lower(?)");
        params.push(V::Text(owner.to_string()));
    }
    let overdue = q.overdue.unwrap_or(false);
    match q.state.as_deref().unwrap_or("open") {
        "all" if !overdue => {}
        "done" if !overdue => sql.push_str(" AND e.done = 1"),
        _ => sql.push_str(" AND e.done = 0"),
    }
    if overdue {
        sql.push_str(" AND e.due GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]*' AND substr(e.due,1,10) < date('now')");
    }
}

fn load_rows(conn: &Connection, kind: &str, q: &RecordQuery) -> Vec<Row> {
    let mut sql = BASE_SQL.to_string();
    let mut params = Vec::new();
    push_filters(&mut sql, &mut params, kind, q);
    if let Some(scope) = resolve_scope(conn, q) {
        if scope.is_empty() {
            return Vec::new();
        }
        sql.push_str(&format!(" AND i.source_id IN ({})", crate::common::placeholders(scope.len())));
        params.extend(scope.into_iter().map(rusqlite::types::Value::Integer));
    }
    sql.push_str(" ORDER BY e.id");
    let Ok(mut stmt) = conn.prepare(&sql) else { return Vec::new() };
    let exclude_disputed = qa_exclude_disputed();
    stmt.query_map(rusqlite::params_from_iter(params.iter()), row_from_sql)
        .map(|rows| rows.flatten().filter(|r| crate::retrieval::status_ok(Some(&r.review_status), exclude_disputed)).collect())
        .unwrap_or_default()
}

fn qa_exclude_disputed() -> bool {
    std::env::var("ANCHOR_QA_EXCLUDE_DISPUTED")
        .map(|v| v != "0" && v.to_lowercase() != "false")
        .unwrap_or(true)
}

/// Inverse supersession: D0012 listing `supersedes: D0005` marks D0005 as
/// superseded by D0012 even when D0005's own record never says so.
fn link_supersession(rows: &mut [Row]) {
    let mut by_target: HashMap<String, String> = HashMap::new();
    for r in rows.iter() {
        for target in &r.supersedes {
            by_target.entry(target.clone()).or_insert_with(|| r.record_id.clone());
        }
    }
    for r in rows.iter_mut() {
        if r.superseded_by.is_empty() && !r.record_id.is_empty() {
            if let Some(newer) = by_target.get(&r.record_id).filter(|n| !n.is_empty()) {
                r.superseded_by = newer.clone();
            }
        }
    }
}

fn effective_status(r: &Row) -> &str {
    if r.superseded_by.is_empty() { &r.record_status } else { "superseded" }
}

fn status_matches(r: &Row, wanted: Option<&str>) -> bool {
    match wanted.map(str::trim).filter(|w| !w.is_empty()) {
        None => true,
        Some(w) if w.eq_ignore_ascii_case("current") => !matches!(effective_status(r), "superseded" | "rejected"),
        Some(w) => effective_status(r).eq_ignore_ascii_case(w),
    }
}

/// Untrusted provider → withhold sensitive/pii-source rows (same rule as
/// `answer::gate_answer_hits`). Returns the blocked labels.
fn gate_rows(rows: &mut Vec<Row>, trusted: bool) -> Vec<String> {
    if trusted {
        return Vec::new();
    }
    let mut blocked = Vec::new();
    rows.retain(|r| {
        let gated = r.label == "sensitive" || r.label == "pii";
        if gated {
            blocked.push(r.label.clone());
        }
        !gated
    });
    blocked
}

fn provenance(r: &Row) -> Value {
    json!({"item_id": r.item_id, "source_id": r.source_id, "path": r.path, "title": r.title, "chunk_id": r.chunk_id})
}

fn decision_json(r: &Row) -> Value {
    json!({
        "entity_id": r.id,
        "record_id": r.record_id,
        "summary": r.summary,
        "status": effective_status(r),
        "current": !matches!(effective_status(r), "superseded" | "rejected"),
        "owner": r.owner,
        "supersedes": r.supersedes,
        "superseded_by": r.superseded_by,
        "review_status": r.review_status,
        "created_at": r.created_at,
        "provenance": provenance(r),
    })
}

fn action_json(r: &Row) -> Value {
    json!({
        "entity_id": r.id,
        "task": r.summary,
        "owner": r.owner,
        "due": r.due,
        "done": r.done,
        "review_status": r.review_status,
        "created_at": r.created_at,
        "provenance": provenance(r),
    })
}

/// One listing, gated + paged. `blocked` carries the withheld labels so the
/// handler can record the gate event outside the blocking section.
pub struct Listing {
    pub body: Value,
    pub blocked: Vec<String>,
}

fn page(rows: Vec<Value>, q: &RecordQuery, key: &str, blocked: &[String]) -> Value {
    let total = rows.len();
    let offset = q.offset.unwrap_or(0);
    let limit = q.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let items: Vec<Value> = rows.into_iter().skip(offset).take(limit).collect();
    let refusal = (!blocked.is_empty() && total == 0).then_some("gated_provider");
    json!({key: items, "total": total, "blocked": blocked.len(), "refusal": refusal})
}

pub fn list_decisions(conn: &Connection, q: &RecordQuery, trusted: bool) -> Listing {
    let mut rows = load_rows(conn, "decision", q);
    link_supersession(&mut rows);
    rows.retain(|r| status_matches(r, q.status.as_deref()));
    let blocked = gate_rows(&mut rows, trusted);
    rows.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(b.id.cmp(&a.id)));
    let items = rows.iter().map(decision_json).collect();
    Listing { body: page(items, q, "decisions", &blocked), blocked }
}

/// ISO-dated actions first (soonest due), then undated, then by id.
fn action_order(a: &Row, b: &Row) -> std::cmp::Ordering {
    let key = |r: &Row| (!is_iso_date(&r.due), r.due.clone(), r.id);
    key(a).cmp(&key(b))
}

fn is_iso_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 10 && b[4] == b'-' && b[7] == b'-' && s[..4].bytes().chain(s[5..7].bytes()).chain(s[8..10].bytes()).all(|c| c.is_ascii_digit())
}

pub fn list_actions(conn: &Connection, q: &RecordQuery, trusted: bool) -> Listing {
    let mut rows = load_rows(conn, "action", q);
    let blocked = gate_rows(&mut rows, trusted);
    rows.sort_by(action_order);
    let items = rows.iter().map(action_json).collect();
    Listing { body: page(items, q, "actions", &blocked), blocked }
}

async fn serve(state: AppState, q: RecordQuery, list: fn(&Connection, &RecordQuery, bool) -> Listing) -> Json<Value> {
    let trusted = crate::answer::is_answer_trusted(&state.settings);
    let data_dir = state.data_dir.clone();
    let listing = tokio::task::spawn_blocking(move || {
        let conn = crate::common::open_data_db(&data_dir);
        list(&conn, &q, trusted)
    })
    .await;
    let Ok(listing) = listing else {
        return Json(json!({"detail": "listing failed"}));
    };
    if !listing.blocked.is_empty() {
        crate::answer::record_gate_event(&state.data_dir, &listing.blocked).await;
    }
    Json(listing.body)
}

pub async fn decisions_handler(State(state): State<AppState>, Query(q): Query<RecordQuery>) -> Json<Value> {
    serve(state, q, list_decisions).await
}

pub async fn actions_handler(State(state): State<AppState>, Query(q): Query<RecordQuery>) -> Json<Value> {
    serve(state, q, list_actions).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tempfile::tempdir;

    struct Fixture {
        _dir: tempfile::TempDir,
        state: AppState,
        conn: Connection,
        sensitive: i64,
    }

    fn insert_item(conn: &Connection, sid: i64, path: &str) -> i64 {
        conn.execute(
            "INSERT INTO ingested_items (source_id, external_id, content_hash, title, text) VALUES (?1,?2,'h',?2,'body')",
            rusqlite::params![sid, path],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn insert_record(conn: &Connection, item: i64, kind: &str, summary: &str, cols: &[(&str, &str)]) -> i64 {
        conn.execute("INSERT INTO entities (item_id, kind, summary) VALUES (?1,?2,?3)", rusqlite::params![item, kind, summary])
            .unwrap();
        let id = conn.last_insert_rowid();
        for (col, val) in cols {
            conn.execute(&format!("UPDATE entities SET {col} = ?1 WHERE id = ?2"), rusqlite::params![val, id]).unwrap();
        }
        id
    }

    /// Two sources: `team` (internal) with D0005 superseded by D0012 and
    /// three actions; `vault` (sensitive) with one decision + one action.
    fn fixture(untrusted: bool) -> Fixture {
        let dir = tempdir().unwrap();
        let store = crate::secrets::SecretStore::new(dir.path().join("secrets.enc"));
        let settings = Arc::new(crate::settings::SettingsService::new(store));
        settings.set_data_dir(dir.path());
        let db_path = crate::db::resolve_db_path(&dir.path().to_string_lossy());
        let conn = crate::db::init_db(&db_path).unwrap();
        let base = if untrusted { "https://untrusted.example.invalid/v1" } else { "http://127.0.0.1:11434/v1" };
        settings.set("answer_base_url", base, &conn).unwrap();
        conn.execute("INSERT INTO sources (connector, name, label) VALUES ('folder','team','internal')", []).unwrap();
        let team = conn.last_insert_rowid();
        conn.execute("INSERT INTO sources (connector, name, label) VALUES ('folder','vault','sensitive')", []).unwrap();
        let sensitive = conn.last_insert_rowid();
        let old = insert_item(&conn, team, "decisions/d5.md");
        let new = insert_item(&conn, team, "decisions/d12.md");
        let retro = insert_item(&conn, team, "meetings/retro.md");
        let vault = insert_item(&conn, sensitive, "vault/moonvault.md");
        insert_record(&conn, old, "decision", "Stripe provider", &[("record_id", "D0005"), ("record_status", "accepted"), ("owner", "Maya")]);
        insert_record(&conn, new, "decision", "Adyen provider", &[("record_id", "D0012"), ("record_status", "accepted"), ("supersedes", "D0005")]);
        insert_record(&conn, retro, "action", "draft spec", &[("owner", "Alex"), ("due", "2000-01-02")]);
        insert_record(&conn, retro, "action", "sign contract", &[("owner", "alex"), ("done", "1")]);
        insert_record(&conn, retro, "action", "close alerting gaps", &[("owner", "Mike")]);
        insert_record(&conn, vault, "decision", "moonvault sealed", &[("record_status", "accepted")]);
        insert_record(&conn, vault, "action", "moonvault rotate keys", &[("owner", "Alex")]);
        let embedder = Arc::new(crate::embedder::Embedder::new(settings.clone()));
        let state = AppState {
            data_dir: dir.path().to_string_lossy().to_string(),
            settings,
            jobs: crate::jobs::JobManager::new(),
            csrf_token: String::new(),
            embedder,
        };
        Fixture { _dir: dir, state, conn, sensitive }
    }

    fn tasks(v: &Value) -> Vec<String> {
        v["actions"].as_array().unwrap().iter().map(|a| a["task"].as_str().unwrap().to_string()).collect()
    }

    #[tokio::test]
    async fn rest_open_actions_by_owner() {
        let f = fixture(false);
        let q = RecordQuery { owner: Some("ALEX".into()), ..Default::default() };
        let Json(v) = actions_handler(State(f.state.clone()), Query(q)).await;
        assert_eq!(tasks(&v), ["draft spec", "moonvault rotate keys"], "open only, owner case-insensitive, dated first");
        let a = &v["actions"][0];
        assert_eq!(a["due"], "2000-01-02");
        assert_eq!(a["provenance"]["path"], "meetings/retro.md");
        assert!(a["provenance"]["item_id"].as_i64().is_some());
        let q = RecordQuery { owner: Some("alex".into()), state: Some("done".into()), ..Default::default() };
        let Json(v) = actions_handler(State(f.state.clone()), Query(q)).await;
        assert_eq!(tasks(&v), ["sign contract"]);
        let q = RecordQuery { overdue: Some(true), ..Default::default() };
        let Json(v) = actions_handler(State(f.state), Query(q)).await;
        assert_eq!(tasks(&v), ["draft spec"], "only ISO-dated open items before today");
    }

    #[tokio::test]
    async fn rest_decisions_show_supersession() {
        let f = fixture(false);
        let Json(v) = decisions_handler(State(f.state.clone()), Query(RecordQuery::default())).await;
        let d5 = v["decisions"].as_array().unwrap().iter().find(|d| d["record_id"] == "D0005").unwrap().clone();
        assert_eq!(d5["status"], "superseded");
        assert_eq!(d5["superseded_by"], "D0012");
        assert_eq!(d5["current"], false);
        let q = RecordQuery { status: Some("current".into()), q: Some("provider".into()), ..Default::default() };
        let Json(v) = decisions_handler(State(f.state), Query(q)).await;
        let ids: Vec<&str> = v["decisions"].as_array().unwrap().iter().map(|d| d["record_id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["D0012"]);
        assert_eq!(v["decisions"][0]["supersedes"], json!(["D0005"]));
    }

    #[tokio::test]
    async fn gate_and_scope_mirror_search() {
        // Untrusted provider: /qa/search withholds the sensitive source —
        // so do decisions/actions, with the same refusal + blocked count.
        let f = fixture(true);
        let req = crate::answer::SearchRequest {
            query: "moonvault".into(), k: Some(8), project_id: None, public_only: None, fields: None,
            cursor: None, limit: None, include_content: None, env: None, kind: None, path_prefix: None,
            symbol: None, mode: None,
        };
        // Index the sensitive item so search has something to withhold.
        let vault_item: i64 = f.conn.query_row("SELECT id FROM ingested_items WHERE external_id='vault/moonvault.md'", [], |r| r.get(0)).unwrap();
        f.conn.execute("INSERT INTO chunks (item_id, kind, source_ref, content) VALUES (?1,'document','vault/moonvault.md','moonvault sealed text')", [vault_item]).unwrap();
        let search = crate::answer::search(&f.state.settings, &f.state.embedder, req, &f.state.data_dir).await;
        assert_eq!(search.refusal.as_deref(), Some("gated_provider"));
        let q = RecordQuery { q: Some("moonvault".into()), ..Default::default() };
        let Json(d) = decisions_handler(State(f.state.clone()), Query(q.clone())).await;
        let Json(a) = actions_handler(State(f.state.clone()), Query(q)).await;
        for v in [&d, &a] {
            assert_eq!(v["refusal"], "gated_provider", "{v}");
            assert_eq!(v["blocked"], 1);
            assert_eq!(v["total"], 0);
        }
        // Trusted path + explicit scope: the sensitive source is listable,
        // and a source_ids filter outside the scope yields nothing.
        let t = fixture(false);
        let q = RecordQuery { source_ids: Some(t.sensitive.to_string()), ..Default::default() };
        let Json(v) = actions_handler(State(t.state.clone()), Query(q)).await;
        assert_eq!(tasks(&v), ["moonvault rotate keys"]);
        let q = RecordQuery { source_ids: Some(t.sensitive.to_string()), public_only: Some(true), ..Default::default() };
        let Json(v) = actions_handler(State(t.state), Query(q)).await;
        assert_eq!(v["total"], 0, "public_only intersects like search");
    }

    #[tokio::test]
    async fn stale_rows_drop_like_search() {
        let f = fixture(false);
        f.conn.execute("UPDATE entities SET status='stale' WHERE summary='close alerting gaps'", []).unwrap();
        let Json(v) = actions_handler(State(f.state), Query(RecordQuery { owner: Some("Mike".into()), ..Default::default() })).await;
        assert_eq!(v["total"], 0);
    }
}
