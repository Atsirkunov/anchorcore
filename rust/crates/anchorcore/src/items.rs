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

fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
}

/// Metadata listing over ingested items — the discovery primitive that ends
/// id-scanning loops (1.0.24 retest #7: ~150 `GET /items/:id` probing ids
/// 200–300 to find `reportdata/` files). `path_prefix` matches the item path
/// start, slash-normalized so `reportdata/` also finds Windows `\` rows.
/// Metadata only (no bytes), so no provider gate — like the entities list.
fn list_items(
    conn: &rusqlite::Connection,
    path_prefix: Option<&str>,
    source_id: Option<i64>,
    limit: i64,
    offset: i64,
) -> (Vec<Value>, i64) {
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
}
