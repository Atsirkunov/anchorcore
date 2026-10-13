//! Whole-item fetch — full file bytes in one call.
//!
//! Search hits and citations carry bounded context (`content_truncated`
//! says when expansion drops siblings); `GET /items/:id` is the drill-down
//! that returns the complete ingested text.
//!
//! R35.1: items carry the manual PII mark (whole document, people-driven).
//! `PATCH /items/:id {"is_pii": bool}` sets/clears it. The old
//! `?gate=provider` honor system is gone with the auto-detect machinery.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::health::AppState;

#[derive(Deserialize)]
pub struct UpdateItemPayload {
    pub is_pii: Option<bool>,
}

#[derive(Deserialize)]
pub struct ListQuery {
    pub path_prefix: Option<String>,
    pub source_id: Option<i64>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

fn item_json(id: i64, source_id: i64, title: String, path: String, text: String, is_pii: bool) -> Value {
    // `content` aliases `text`: every content-carrying payload exposes
    // `content` (search hits, citations), so item reads do too (1.0.24
    // retest #8). `text` stays the canonical key for backward compat.
    json!({"id": id, "source_id": source_id, "title": title, "path": path, "text": text.clone(), "content": text, "is_pii": is_pii})
}

pub(crate) fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
}

/// Slash-normalize a filesystem path: `\` → `/`, drive letter lowercased.
/// Trailing slashes are preserved — `dir/` and `dir` differ as LIKE prefixes.
pub(crate) fn slash_norm(p: &str) -> String {
    let mut n = p.replace('\\', "/");
    if n.len() >= 2 && n.as_bytes()[1] == b':' && n.as_bytes()[0].is_ascii_alphabetic() {
        n.replace_range(0..1, &n[0..1].to_lowercase());
    }
    n
}

/// True for absolute filesystem prefixes: `/...`, `C:/...`, UNC `\\...`
/// (which slash-normalizes to `//...`). Drive-relative `C:x` reads as relative.
pub(crate) fn is_absolute_prefix(p: &str) -> bool {
    let n = slash_norm(p);
    n.starts_with('/') || (n.len() > 2 && n.as_bytes()[1] == b':' && n.as_bytes()[2] == b'/')
}

/// Folder source roots from stored configs: source_id → normalized root
/// (no trailing slash; `/` survives as itself). Non-folder sources and
/// configs without a path are skipped — absolute prefixes only resolve
/// through folder roots.
pub(crate) fn folder_roots(conn: &rusqlite::Connection) -> std::collections::HashMap<i64, String> {
    let mut out = std::collections::HashMap::new();
    let mut stmt = match conn.prepare("SELECT id, config FROM sources WHERE connector = 'folder'") {
        Ok(s) => s,
        Err(_) => return out,
    };
    let rows: Vec<(i64, String)> = match stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))) {
        Ok(m) => m.filter_map(|r| r.ok()).collect(),
        Err(_) => return out,
    };
    for (sid, cfg) in rows {
        let path = serde_json::from_str::<serde_json::Value>(&cfg)
            .ok()
            .and_then(|v| v.get("path").and_then(|p| p.as_str()).map(|s| s.to_string()))
            .unwrap_or_default();
        if path.trim().is_empty() {
            continue;
        }
        let trimmed = slash_norm(path.trim()).trim_end_matches('/').to_string();
        out.insert(sid, if trimmed.is_empty() { "/".to_string() } else { trimmed });
    }
    out
}

/// Resolve an absolute prefix through folder roots: one entry per matching
/// source, with the relative remainder (`None` = the whole source — the
/// prefix is the root itself or an ancestor of it). `source_id` scopes the
/// candidates. Trailing slashes survive into the remainder so `dir/` keeps
/// its LIKE precision.
pub(crate) fn absolute_remainders(
    roots: &std::collections::HashMap<i64, String>,
    prefix: &str,
    source_id: Option<i64>,
) -> Vec<(i64, Option<String>)> {
    let raw = slash_norm(prefix);
    let trimmed = raw.trim_end_matches('/');
    let p = if trimmed.is_empty() { "/" } else { trimmed };
    let mut out = Vec::new();
    for (sid, root) in roots {
        if source_id.is_some_and(|want| *sid != want) {
            continue;
        }
        if p == "/" || root == p || root.starts_with(&format!("{p}/")) {
            out.push((*sid, None));
            continue;
        }
        if let Some(rel) = raw.strip_prefix(&format!("{root}/")) {
            out.push((*sid, Some(rel.to_string())));
        }
    }
    out.sort_by_key(|(sid, _)| *sid);
    out
}

/// One item against a listing/search prefix. Relative prefixes match the
/// stored external id start (slash-normalized); absolute prefixes resolve
/// through `absolute_remainders`, so this mirrors the listing query exactly
/// (LIKE-prefix == case-folded `starts_with`).
pub(crate) fn item_under_prefix(
    external_id: &str,
    source_id: Option<i64>,
    roots: &std::collections::HashMap<i64, String>,
    prefix: &str,
) -> bool {
    if !is_absolute_prefix(prefix) {
        return external_id.replace('\\', "/").to_lowercase().starts_with(&slash_norm(prefix).to_lowercase());
    }
    let sid = match source_id {
        Some(s) => s,
        None => return false,
    };
    for (rsid, rel) in absolute_remainders(roots, prefix, Some(sid)) {
        if rsid != sid {
            continue;
        }
        return match rel {
            None => true,
            Some(r) => external_id.replace('\\', "/").to_lowercase().starts_with(&r.to_lowercase()),
        };
    }
    false
}

/// Metadata listing over ingested items — the discovery primitive that ends
/// id-scanning loops (1.0.24 retest #7: ~150 `GET /items/:id` probing ids
/// 200–300 to find `reportdata/` files). `path_prefix` matches the item path
/// start, slash-normalized so `reportdata/` also finds Windows `\` rows.
/// Relative prefixes match the stored external id; absolute prefixes resolve
/// through the owning folder source's root (R36.6).
/// Metadata only (no bytes), so no provider gate — like the entities list.
fn list_items(
    conn: &rusqlite::Connection,
    path_prefix: Option<&str>,
    source_id: Option<i64>,
    limit: i64,
    offset: i64,
) -> (Vec<Value>, i64) {
    if let Some(p) = path_prefix {
        if is_absolute_prefix(p) {
            return list_items_absolute(conn, p, source_id, limit, offset);
        }
    }
    let mut conds: Vec<String> = Vec::new();
    let mut params: Vec<rusqlite::types::Value> = Vec::new();
    if let Some(p) = path_prefix {
        let norm = p.replace('\\', "/");
        conds.push("REPLACE(external_id, '\\', '/') LIKE ? ESCAPE '\\'".to_string());
        params.push(rusqlite::types::Value::Text(format!("{}%", escape_like(&norm))));
    }
    if let Some(sid) = source_id {
        conds.push("source_id = ?".to_string());
        params.push(rusqlite::types::Value::Integer(sid));
    }
    run_list_query(conn, conds, params, limit, offset)
}

/// Absolute-prefix listing: the prefix resolves to per-source relative
/// remainders (`absolute_remainders`), each a `source_id` + LIKE arm. A
/// prefix covering no root matches nothing — never everything.
fn list_items_absolute(
    conn: &rusqlite::Connection,
    prefix: &str,
    source_id: Option<i64>,
    limit: i64,
    offset: i64,
) -> (Vec<Value>, i64) {
    let roots = folder_roots(conn);
    let mut ors: Vec<String> = Vec::new();
    let mut params: Vec<rusqlite::types::Value> = Vec::new();
    for (sid, rel) in absolute_remainders(&roots, prefix, source_id) {
        match rel {
            None => {
                ors.push("source_id = ?".to_string());
                params.push(rusqlite::types::Value::Integer(sid));
            }
            Some(r) => {
                ors.push("(source_id = ? AND REPLACE(external_id, '\\', '/') LIKE ? ESCAPE '\\')".to_string());
                params.push(rusqlite::types::Value::Integer(sid));
                params.push(rusqlite::types::Value::Text(format!("{}%", escape_like(&r))));
            }
        }
    }
    if ors.is_empty() {
        return run_list_query(conn, vec!["1 = 0".to_string()], params, limit, offset);
    }
    run_list_query(conn, vec![format!("({})", ors.join(" OR "))], params, limit, offset)
}

fn run_list_query(
    conn: &rusqlite::Connection,
    conds: Vec<String>,
    mut params: Vec<rusqlite::types::Value>,
    limit: i64,
    offset: i64,
) -> (Vec<Value>, i64) {
    let filter = if conds.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", conds.join(" AND "))
    };
    let total: i64 = conn
        .query_row(
            &format!("SELECT COUNT(*) FROM ingested_items {}", filter),
            rusqlite::params_from_iter(params.iter()),
            |r| r.get(0),
        )
        .unwrap_or(0);
    let sql = format!(
        "SELECT id, source_id, title, external_id, is_pii FROM ingested_items {} ORDER BY external_id, id LIMIT ? OFFSET ?",
        filter
    );
    params.push(rusqlite::types::Value::Integer(limit));
    params.push(rusqlite::types::Value::Integer(offset));
    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(_) => return (vec![], total),
    };
    let rows = stmt
        .query_map(rusqlite::params_from_iter(params.iter()), |r| {
            Ok(json!({
                "id": r.get::<_, i64>(0)?,
                "source_id": r.get::<_, i64>(1)?,
                "title": r.get::<_, String>(2)?,
                "path": r.get::<_, String>(3)?,
                "is_pii": r.get::<_, i64>(4)? != 0,
            }))
        })
        .map(|m| m.filter_map(|r| r.ok()).collect())
        .unwrap_or_default();
    (rows, total)
}

pub async fn list_handler(State(state): State<AppState>, Query(q): Query<ListQuery>) -> Json<Value> {
    let data_dir = state.data_dir.clone();
    let result = tokio::task::spawn_blocking(move || {
        let conn = crate::common::open_data_db(&data_dir);
        let limit = q.limit.unwrap_or(100).clamp(1, 1000);
        let offset = q.offset.unwrap_or(0).max(0);
        let (items, total) = list_items(&conn, q.path_prefix.as_deref(), q.source_id, limit, offset);
        json!({"items": items, "total": total})
    })
    .await
    .unwrap();
    Json(result)
}

fn fetch_item(conn: &rusqlite::Connection, item_id: i64) -> Option<Value> {
    let row: Option<(i64, i64, String, String, String, bool)> = conn
        .query_row(
            "SELECT id, source_id, title, external_id, text, is_pii FROM ingested_items WHERE id = ?1",
            [item_id],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get::<_, Option<String>>(4)?.unwrap_or_default(),
                    r.get::<_, i64>(5)? != 0,
                ))
            },
        )
        .ok();
    row.map(|(id, sid, title, path, text, is_pii)| item_json(id, sid, title, path, text, is_pii))
}

pub async fn get_handler(
    State(state): State<AppState>,
    Path(item_id): Path<i64>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let data_dir = state.data_dir.clone();
    let result = tokio::task::spawn_blocking(move || {
        let conn = crate::common::open_data_db(&data_dir);
        match fetch_item(&conn, item_id) {
            Some(v) => Ok(v),
            None => Err((StatusCode::NOT_FOUND, Json(json!({"detail": "Item not found"})))),
        }
    })
    .await
    .unwrap();
    match result {
        Ok(v) => Ok(Json(v)),
        Err(e) => Err(e),
    }
}

pub async fn patch_handler(
    State(state): State<AppState>,
    Path(item_id): Path<i64>,
    Json(payload): Json<UpdateItemPayload>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let data_dir = state.data_dir.clone();
    let result = tokio::task::spawn_blocking(move || {
        let conn = crate::common::open_data_db(&data_dir);
        crate::common::require_row(&conn, "ingested_items", item_id, "Item not found")?;
        if let Some(mark) = payload.is_pii {
            let _ = conn.execute(
                "UPDATE ingested_items SET is_pii = ?1, updated_at = datetime('now') WHERE id = ?2",
                rusqlite::params![i64::from(mark), item_id],
            );
        }
        match fetch_item(&conn, item_id) {
            Some(v) => Ok(v),
            None => Err((StatusCode::NOT_FOUND, Json(json!({"detail": "Item not found"})))),
        }
    })
    .await
    .unwrap();
    match result {
        Ok(v) => Ok(Json(v)),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tempfile::tempdir;

    fn seed_db() -> (tempfile::TempDir, rusqlite::Connection, i64) {
        let dir = tempdir().unwrap();
        let db_path = crate::db::resolve_db_path(&dir.path().to_string_lossy().to_string());
        let conn = crate::db::init_db(&db_path).unwrap();
        conn.execute("INSERT INTO sources (name, connector) VALUES ('s','folder')", []).unwrap();
        let sid = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO ingested_items (source_id, external_id, title, text, content_hash) VALUES (?1,'cfg/routing-config.yml','routing-config.yml','ACME_FEE: x','h')",
            [sid],
        )
        .unwrap();
        let iid = conn.last_insert_rowid();
        (dir, conn, iid)
    }

    fn app_state(dir: &tempfile::TempDir) -> AppState {
        let store = crate::secrets::SecretStore::new(dir.path().join("secrets.enc"));
        let settings = Arc::new(crate::settings::SettingsService::new(store));
        settings.set_data_dir(dir.path());
        let embedder = Arc::new(crate::embedder::Embedder::new(settings.clone()));
        AppState {
            data_dir: dir.path().to_string_lossy().to_string(),
            settings,
            jobs: crate::jobs::JobManager::new(),
            csrf_token: String::new(),
            embedder,
        }
    }

    #[test]
    fn fetch_item_returns_full_text_with_path() {
        let (_dir, conn, iid) = seed_db();
        let v = fetch_item(&conn, iid).expect("seeded item must fetch");
        assert_eq!(v.get("path").and_then(|x| x.as_str()), Some("cfg/routing-config.yml"));
        assert_eq!(v.get("text").and_then(|x| x.as_str()), Some("ACME_FEE: x"));
        assert_eq!(v.get("content").and_then(|x| x.as_str()), Some("ACME_FEE: x"), "`content` aliases `text` (hits/citations parity)");
        assert_eq!(v.get("is_pii").and_then(|x| x.as_bool()), Some(false), "new items default unmarked");
        assert!(fetch_item(&conn, iid + 99).is_none(), "missing item reads as None (404)");
    }

    fn seed_list_db() -> (tempfile::TempDir, rusqlite::Connection, i64, i64) {
        let dir = tempdir().unwrap();
        let db_path = crate::db::resolve_db_path(&dir.path().to_string_lossy().to_string());
        let conn = crate::db::init_db(&db_path).unwrap();
        conn.execute("INSERT INTO sources (name, connector) VALUES ('a','folder')", []).unwrap();
        let sa = conn.last_insert_rowid();
        conn.execute("INSERT INTO sources (name, connector) VALUES ('b','folder')", []).unwrap();
        let sb = conn.last_insert_rowid();
        let paths = [
            (sa, "reportdata/a.yml"),
            (sa, "reportdata/b.yml"),
            (sa, "reportdata\\win.yml"),
            (sb, "reportdata/other.yml"),
            (sa, "cfg/c.yml"),
            (sa, "report%data/x.yml"),
            (sa, "report_data/y.yml"),
        ];
        for (sid, p) in paths {
            conn.execute(
                "INSERT INTO ingested_items (source_id, external_id, title, text, content_hash) VALUES (?1,?2,?2,'t','h')",
                rusqlite::params![sid, p],
            )
            .unwrap();
        }
        (dir, conn, sa, sb)
    }

    fn list_paths(items: &[Value]) -> Vec<String> {
        items.iter().filter_map(|v| v.get("path").and_then(|p| p.as_str()).map(|s| s.to_string())).collect()
    }

    #[test]
    fn list_items_filters_by_path_prefix() {
        let (_dir, conn, sa, sb) = seed_list_db();
        // Plain prefix: forward and backslash rows alike, ordered by path.
        let (items, total) = list_items(&conn, Some("reportdata/"), None, 100, 0);
        assert_eq!(total, 4);
        assert_eq!(list_paths(&items), vec!["reportdata/a.yml", "reportdata/b.yml", "reportdata/other.yml", "reportdata\\win.yml"]);
        assert!(items.iter().all(|v| v.get("text").is_none() && v.get("content").is_none()), "listing is metadata only");
        // LIKE specials in the prefix match literally, never as wildcards.
        let (items, total) = list_items(&conn, Some("report_"), None, 100, 0);
        assert_eq!(total, 1);
        assert_eq!(list_paths(&items), vec!["report_data/y.yml"]);
        let (items, total) = list_items(&conn, Some("report%"), None, 100, 0);
        assert_eq!(total, 1);
        assert_eq!(list_paths(&items), vec!["report%data/x.yml"]);
        // Source scope composes with the prefix.
        let (items, total) = list_items(&conn, Some("reportdata/"), Some(sb), 100, 0);
        assert_eq!(total, 1);
        assert_eq!(list_paths(&items), vec!["reportdata/other.yml"]);
        let (_items, total) = list_items(&conn, None, Some(sa), 100, 0);
        assert_eq!(total, 6);
        // Pagination walks the full set; total ignores the window.
        let (p1, total) = list_items(&conn, None, None, 3, 0);
        let (p2, _) = list_items(&conn, None, None, 3, 3);
        let (p3, _) = list_items(&conn, None, None, 3, 6);
        assert_eq!(total, 7);
        assert_eq!(p1.len() + p2.len() + p3.len(), 7);
        let mut all = list_paths(&p1);
        all.extend(list_paths(&p2));
        all.extend(list_paths(&p3));
        all.sort();
        all.dedup();
        assert_eq!(all.len(), 7, "pages must cover distinct items");
    }

    #[tokio::test]
    async fn list_handler_serves_prefix_query() {
        let (dir, conn, _sa, _sb) = seed_list_db();
        drop(conn);
        let state = app_state(&dir);
        let q = ListQuery { path_prefix: Some("reportdata/".to_string()), source_id: None, limit: Some(2), offset: Some(1) };
        let Json(v) = list_handler(State(state), Query(q)).await;
        assert_eq!(v.get("total").and_then(|t| t.as_i64()), Some(4));
        let items = v.get("items").and_then(|i| i.as_array()).cloned().unwrap_or_default();
        assert_eq!(list_paths(&items), vec!["reportdata/b.yml", "reportdata/other.yml"]);
    }

    #[tokio::test]
    async fn patch_item_pii_mark_round_trip() {
        let dir = tempdir().unwrap();
        let db_path = crate::db::resolve_db_path(&dir.path().to_string_lossy().to_string());
        let conn = crate::db::init_db(&db_path).unwrap();
        conn.execute("INSERT INTO sources (name, connector) VALUES ('s','folder')", []).unwrap();
        let sid = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO ingested_items (source_id, external_id, title, text, content_hash) VALUES (?1,'d/f.md','f.md','body','h')",
            [sid],
        )
        .unwrap();
        let iid = conn.last_insert_rowid();
        drop(conn);
        let state = app_state(&dir);
        let mark = UpdateItemPayload { is_pii: Some(true) };
        let Json(v) = patch_handler(State(state.clone()), Path(iid), Json(mark))
            .await
            .expect("mark must succeed");
        assert_eq!(v.get("is_pii").and_then(|x| x.as_bool()), Some(true));
        let Json(got) = get_handler(State(state.clone()), Path(iid)).await.expect("get must succeed");
        assert_eq!(got.get("is_pii").and_then(|x| x.as_bool()), Some(true), "mark persists");
        let clear = UpdateItemPayload { is_pii: Some(false) };
        let Json(v2) = patch_handler(State(state.clone()), Path(iid), Json(clear))
            .await
            .expect("clear must succeed");
        assert_eq!(v2.get("is_pii").and_then(|x| x.as_bool()), Some(false));
        // Missing item → 404, not a silent no-op.
        let miss = UpdateItemPayload { is_pii: Some(true) };
        match patch_handler(State(state), Path(iid + 99), Json(miss)).await {
            Err((s, _)) => assert_eq!(s, StatusCode::NOT_FOUND),
            Ok(_) => panic!("patch on missing item must 404"),
        }
    }

    fn seed_abs_db() -> (tempfile::TempDir, tempfile::TempDir, rusqlite::Connection, i64, i64, String) {
        let dbdir = tempdir().unwrap();
        let corpus = tempdir().unwrap();
        let db_path = crate::db::resolve_db_path(&dbdir.path().to_string_lossy().to_string());
        let conn = crate::db::init_db(&db_path).unwrap();
        let root = corpus.path().to_string_lossy().to_string();
        let cfg = serde_json::json!({"path": root}).to_string();
        conn.execute("INSERT INTO sources (name, connector, config) VALUES ('docs','folder',?1)", [cfg]).unwrap();
        let sf: i64 = conn.query_row("SELECT id FROM sources WHERE name='docs'", [], |r| r.get(0)).unwrap();
        conn.execute("INSERT INTO sources (name, connector) VALUES ('jira','jira')", []).unwrap();
        let sj: i64 = conn.query_row("SELECT id FROM sources WHERE name='jira'", [], |r| r.get(0)).unwrap();
        for (sid, p) in [
            (sf, "reportdata/a.yml"),
            (sf, "reportdata\\win.yml"),
            (sf, "cfg/c.yml"),
            (sf, "report%data/x.yml"),
            (sj, "PROJ-1"),
        ] {
            conn.execute(
                "INSERT INTO ingested_items (source_id, external_id, title, text, content_hash) VALUES (?1,?2,?2,'t','h')",
                rusqlite::params![sid, p],
            )
            .unwrap();
        }
        (dbdir, corpus, conn, sf, sj, root)
    }

    #[test]
    fn list_items_accepts_absolute_prefixes() {
        let (_dbdir, _corpus, conn, sf, sj, root) = seed_abs_db();
        // The root itself lists the whole folder source (never the jira row).
        let (items, total) = list_items(&conn, Some(&root), None, 100, 0);
        assert_eq!(total, 4);
        assert!(list_paths(&items).iter().all(|p| *p != "PROJ-1"));
        // A subdirectory narrows to it, backslash rows included.
        let sub = format!("{root}/reportdata");
        let (items, total) = list_items(&conn, Some(&sub), None, 100, 0);
        assert_eq!(total, 2);
        assert_eq!(list_paths(&items), vec!["reportdata/a.yml", "reportdata\\win.yml"]);
        // Trailing slash + backslash separators spell the same prefix.
        let (items, total) = list_items(&conn, Some(&format!("{sub}/")), None, 100, 0);
        assert_eq!(total, 2);
        assert_eq!(list_paths(&items).len(), 2);
        let (items, total) = list_items(&conn, Some(&sub.replace('/', "\\")), None, 100, 0);
        assert_eq!(total, 2, "backslash absolute prefix must match: {}", sub);
        assert_eq!(list_paths(&items).len(), 2);
        // LIKE specials in the absolute tail match literally.
        let (items, total) = list_items(&conn, Some(&format!("{root}/report%")), None, 100, 0);
        assert_eq!(total, 1);
        assert_eq!(list_paths(&items), vec!["report%data/x.yml"]);
        // source_id composes: another source's root prefix finds nothing here.
        let (_items, total) = list_items(&conn, Some(&sub), Some(sj), 100, 0);
        assert_eq!(total, 0, "absolute prefix on a non-folder source matches nothing");
        let (_items, total) = list_items(&conn, Some(&sub), Some(sf), 100, 0);
        assert_eq!(total, 2);
        // An ancestor of the root covers the whole source; an unrelated
        // absolute path matches nothing — never everything.
        let parent = std::path::Path::new(&root).parent().unwrap().to_string_lossy().to_string();
        let (_items, total) = list_items(&conn, Some(&parent), Some(sf), 100, 0);
        assert_eq!(total, 4);
        let (_items, total) = list_items(&conn, Some("/no/such/root/anywhere"), None, 100, 0);
        assert_eq!(total, 0);
        let (items, total) = list_items(&conn, Some("/no/such/root/anywhere"), None, 1, 0);
        assert_eq!(total, 0);
        assert!(items.is_empty());
    }

    #[test]
    fn absolute_prefix_helpers() {
        assert!(is_absolute_prefix("/a/b"));
        assert!(is_absolute_prefix("C:/corpus"));
        assert!(is_absolute_prefix("c:\\corpus\\sub"));
        assert!(is_absolute_prefix("\\\\srv\\share"));
        assert!(!is_absolute_prefix("reportdata/"));
        assert!(!is_absolute_prefix("C:relative"));
        assert!(!is_absolute_prefix(""));
        assert_eq!(slash_norm("C:\\A\\b"), "c:/A/b");
        let mut roots = std::collections::HashMap::new();
        roots.insert(1, "/data/corpus".to_string());
        roots.insert(2, "/data/other".to_string());
        // Root itself + ancestor cover the whole source; segment boundaries hold.
        assert_eq!(absolute_remainders(&roots, "/data/corpus", None), vec![(1, None)]);
        assert_eq!(absolute_remainders(&roots, "/data", None), vec![(1, None), (2, None)]);
        assert!(absolute_remainders(&roots, "/data/corp", None).is_empty(), "no segment-boundary bleed");
        assert_eq!(
            absolute_remainders(&roots, "/data/corpus/reportdata/", None),
            vec![(1, Some("reportdata/".to_string()))]
        );
        assert!(absolute_remainders(&roots, "/data/corpus/x", Some(2)).is_empty());
        // item_under_prefix mirrors the listing: relative, absolute, ancestor,
        // boundary, case-folded LIKE parity, and missing-root reads.
        assert!(item_under_prefix("reportdata/a.yml", Some(1), &roots, "reportdata/"));
        assert!(!item_under_prefix("cfg/c.yml", Some(1), &roots, "reportdata/"));
        assert!(item_under_prefix("reportdata/a.yml", Some(1), &roots, "/data/corpus/reportdata"));
        assert!(item_under_prefix("reportdata\\a.yml", Some(1), &roots, "/data/corpus/reportdata/"));
        assert!(item_under_prefix("cfg/c.yml", Some(1), &roots, "/data"));
        assert!(!item_under_prefix("f", Some(1), &roots, "/data/corp"));
        assert!(item_under_prefix("ReportData/a.yml", Some(1), &roots, "reportdata/"));
        assert!(!item_under_prefix("reportdata/a.yml", Some(9), &roots, "/data/corpus/reportdata"));
        assert!(!item_under_prefix("reportdata/a.yml", None, &roots, "/data/corpus/reportdata"));
    }
}
