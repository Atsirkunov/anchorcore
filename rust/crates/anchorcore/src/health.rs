use axum::{extract::State, Json};
use rusqlite::Connection;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Mutex<Connection>>,
    pub data_dir: String,
}

pub async fn health(State(state): State<AppState>) -> Json<Value> {
    let (failing_sources, pending_embeddings) = {
        let conn = state.db.lock().unwrap();
        let failing = failing_sources(&conn);
        let pending: i64 = conn
            .query_row("SELECT COUNT(*) FROM chunks WHERE embedding IS NULL", [], |r| r.get(0))
            .unwrap_or(0);
        (failing, pending)
    };

    Json(json!({
        "status": "ok",
        "data_dir": state.data_dir,
        "components": {
            "ollama": "offline", // R1.2 does not yet probe Ollama; match Python shape
            "answer_key": null,
            "pending_embeddings": pending_embeddings,
            "tasks": {},
            "classifier": { "concurrency": 4 },
            "failing_sources": failing_sources
        }
    }))
}

fn failing_sources(conn: &Connection) -> Vec<Value> {
    let mut stmt = match conn.prepare(
        "SELECT id, name, last_error, error_count FROM sources WHERE error_count > 0 AND enabled = 1",
    ) {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    let rows = stmt
        .query_map([], |r| {
            Ok(json!({
                "id": r.get::<_, i64>(0)?,
                "name": r.get::<_, String>(1)?,
                "error": r.get::<_, Option<String>>(2)?,
                "count": r.get::<_, i64>(3)?
            }))
        })
        .unwrap();
    rows.filter_map(|r| r.ok()).collect()
}
