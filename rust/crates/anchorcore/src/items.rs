//! Whole-item fetch — full file bytes in one call.
//!
//! Search hits and citations carry bounded context (`content_truncated`
//! says when expansion dropped siblings); `GET /items/:id` is the drill-down
//! that returns the complete ingested text. Same provider gate as entities:
//! callers whose reads flow toward an agent/model pass `?gate=provider`.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde_json::{json, Value};

use crate::entities::GateQuery;
use crate::health::AppState;

fn item_json(id: i64, source_id: i64, title: String, path: String, text: String) -> Value {
    json!({"id": id, "source_id": source_id, "title": title, "path": path, "text": text})
}

fn fetch_item(conn: &rusqlite::Connection, item_id: i64) -> Option<Value> {
    let row: Option<(i64, i64, String, String, String)> = conn
        .query_row(
            "SELECT id, source_id, title, external_id, text FROM ingested_items WHERE id = ?1",
            [item_id],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get::<_, Option<String>>(4)?.unwrap_or_default(),
                ))
            },
        )
        .ok();
    row.map(|(id, sid, title, path, text)| item_json(id, sid, title, path, text))
}

pub async fn get_handler(
    State(state): State<AppState>,
    Path(item_id): Path<i64>,
    Query(gate): Query<GateQuery>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let data_dir = state.data_dir.clone();
    let settings = state.settings.clone();
    let result = tokio::task::spawn_blocking(move || {
        let conn = crate::common::open_data_db(&data_dir);
        if gate.gate.as_deref() == Some("provider") && !crate::answer::is_answer_trusted(&settings) {
            if let Some(reason) = crate::answer::gate_item_content(&conn, item_id) {
                let _ = conn.execute(
                    "INSERT INTO system_events (component, level, message, detail) VALUES ('items','warning','item read gated (provider)',?1)",
                    rusqlite::params![reason],
                );
                return Err((
                    StatusCode::FORBIDDEN,
                    Json(json!({"code": "gated_provider", "detail": reason})),
                ));
            }
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

    #[test]
    fn fetch_item_returns_full_text_with_path() {
        let (_dir, conn, iid) = seed_db();
        let v = fetch_item(&conn, iid).expect("seeded item must fetch");
        assert_eq!(v.get("path").and_then(|x| x.as_str()), Some("cfg/routing-config.yml"));
        assert_eq!(v.get("text").and_then(|x| x.as_str()), Some("ACME_FEE: x"));
        assert!(fetch_item(&conn, iid + 99).is_none(), "missing item reads as None (404)");
    }
}
