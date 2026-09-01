//! Sections TOC — R14.1 hierarchical tree.

use axum::{
    extract::{Query, State},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::health::AppState;

#[derive(Deserialize)]
pub struct ListQuery {
    pub item_id: Option<i64>,
    pub source_id: Option<i64>,
}

fn section_row(row: &rusqlite::Row) -> rusqlite::Result<Value> {
    Ok(json!({
        "id": row.get::<_, i64>(0)?,
        "item_id": row.get::<_, i64>(1)?,
        "parent_id": row.get::<_, Option<i64>>(2)?,
        "level": row.get::<_, i64>(3)?,
        "title": row.get::<_, String>(4)?,
        "path": row.get::<_, String>(5)?,
        "chunk_range": row.get::<_, String>(6)?,
        "summary": row.get::<_, String>(7)?,
        "created_at": row.get::<_, Option<String>>(8)?,
    }))
}

pub async fn list_handler(
    State(state): State<AppState>,
    Query(q): Query<ListQuery>,
) -> Json<Value> {
    let data_dir = state.data_dir.clone();
    let item_id = q.item_id;
    let source_id = q.source_id;
    let result = tokio::task::spawn_blocking(move || {
        let db_path = crate::db::resolve_db_path(&data_dir);
        let conn = crate::db::init_db(&db_path)
            .unwrap_or_else(|_| rusqlite::Connection::open(&db_path).unwrap());
        let mut sql = String::from(
            "SELECT id, item_id, parent_id, level, title, path, chunk_range, summary, created_at FROM sections WHERE 1=1",
        );
        let mut params: Vec<rusqlite::types::Value> = Vec::new();
        if let Some(id) = item_id {
            sql.push_str(" AND item_id = ?");
            params.push(rusqlite::types::Value::Integer(id));
        }
        if let Some(sid) = source_id {
            sql.push_str(" AND item_id IN (SELECT id FROM ingested_items WHERE source_id = ?)");
            params.push(rusqlite::types::Value::Integer(sid));
        }
        sql.push_str(" ORDER BY level ASC, id ASC");
        let mut stmt = match conn.prepare(&sql) {
            Ok(s) => s,
            Err(e) => return Json(json!({"error": e.to_string()})),
        };
        let rows = match stmt.query_map(rusqlite::params_from_iter(params.iter()), section_row) {
            Ok(m) => m,
            Err(e) => return Json(json!({"error": e.to_string()})),
        };
        let out: Vec<Value> = rows.filter_map(|r| r.ok()).collect();
        Json(Value::Array(out))
    })
    .await
    .unwrap();
    result
}

pub async fn chunks_handler(
    State(state): State<AppState>,
    axum::extract::Path(section_id): axum::extract::Path<i64>,
) -> Json<Value> {
    let data_dir = state.data_dir.clone();
    let result = tokio::task::spawn_blocking(move || {
        let db_path = crate::db::resolve_db_path(&data_dir);
        let conn = crate::db::init_db(&db_path)
            .unwrap_or_else(|_| rusqlite::Connection::open(&db_path).unwrap());
        let mut stmt = match conn.prepare(
            "SELECT id, item_id, source_ref, content, level, path, created_at FROM chunks WHERE section_id = ?1 ORDER BY id",
        ) {
            Ok(s) => s,
            Err(e) => return Json(json!({"error": e.to_string()})),
        };
        let rows = stmt
            .query_map([section_id], |r| {
                Ok(json!({
                    "id": r.get::<_, i64>(0)?,
                    "item_id": r.get::<_, Option<i64>>(1)?,
                    "source_ref": r.get::<_, String>(2)?,
                    "content": r.get::<_, String>(3)?,
                    "level": r.get::<_, i64>(4)?,
                    "path": r.get::<_, String>(5)?,
                    "created_at": r.get::<_, Option<String>>(6)?,
                }))
            })
            .unwrap();
        let out: Vec<Value> = rows.filter_map(|r| r.ok()).collect();
        Json(Value::Array(out))
    })
    .await
    .unwrap();
    result
}
