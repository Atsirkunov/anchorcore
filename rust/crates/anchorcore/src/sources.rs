//! Sources CRUD — minimal port for MCP `list_sources`/`get_source` + QA scoping.
//! Port of `backend/app/routers/sources.py:1` (subset).

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde_json::Value;

use crate::health::AppState;

fn source_json(row: &rusqlite::Row) -> rusqlite::Result<Value> {
    let id: i64 = row.get(0)?;
    let name: String = row.get(1)?;
    let connector: String = row.get(2)?;
    let config: String = row.get(3)?;
    let enabled: i64 = row.get(4)?;
    let label: String = row.get(5)?;
    let last_synced_at: Option<String> = row.get(6)?;
    let last_error: Option<String> = row.get(7)?;
    let error_count: i64 = row.get(8)?;
    let created_at: Option<String> = row.get(9)?;
    // config is stored as JSON string; we return it as object for detail, but list masks it
    let _cfg: Value = serde_json::from_str(&config).unwrap_or(Value::Object(Default::default()));
    Ok(serde_json::json!({
        "id": id,
        "name": name,
        "connector": connector,
        "enabled": enabled != 0,
        "label": label,
        "last_synced_at": last_synced_at,
        "last_error": last_error,
        "error_count": error_count,
        "created_at": created_at.unwrap_or_default()
    }))
}

pub async fn list_handler(State(state): State<AppState>) -> Json<Value> {
    let data_dir = state.data_dir.clone();
    let result = tokio::task::spawn_blocking(move || {
        let db_path = std::path::PathBuf::from(&data_dir).join("anchorcore.db");
        let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| rusqlite::Connection::open(&db_path).unwrap());
        let mut stmt = conn.prepare("SELECT id, name, connector, config, enabled, label, last_synced_at, last_error, error_count, created_at FROM sources ORDER BY created_at").unwrap();
        let rows = stmt.query_map([], |r| source_json(r)).unwrap();
        let out: Vec<Value> = rows.filter_map(|r| r.ok()).collect();
        Value::Array(out)
    }).await.unwrap();
    Json(result)
}

pub async fn get_handler(State(state): State<AppState>, Path(source_id): Path<i64>) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let data_dir = state.data_dir.clone();
    let result = tokio::task::spawn_blocking(move || {
        let db_path = std::path::PathBuf::from(&data_dir).join("anchorcore.db");
        let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| rusqlite::Connection::open(&db_path).unwrap());
        let mut stmt = conn.prepare("SELECT id, name, connector, config, enabled, label, last_synced_at, last_error, error_count, created_at FROM sources WHERE id = ?1").unwrap();
        match stmt.query_row([source_id], |r| source_json(r)) {
            Ok(v) => Ok(Json(v)),
            Err(_) => Err((StatusCode::NOT_FOUND, Json(serde_json::json!({"detail":"Source not found"})))),
        }
    }).await.unwrap();
    result
}

// POST /sources — minimal create for tests (not full pipeline)
pub async fn create_handler(State(state): State<AppState>, Json(payload): Json<Value>) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)> {
    let data_dir = state.data_dir.clone();
    let name = payload.get("name").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    if name.is_empty() {
        return Err((StatusCode::UNPROCESSABLE_ENTITY, Json(serde_json::json!({"detail":"name required"}))));
    }
    let connector = payload.get("connector").and_then(|v| v.as_str()).unwrap_or("folder").to_string();
    let label = payload.get("label").and_then(|v| v.as_str()).unwrap_or("internal").to_string();
    let config_val = payload.get("config").cloned().unwrap_or(Value::Object(Default::default()));
    let config_str = serde_json::to_string(&config_val).unwrap();
    let result = tokio::task::spawn_blocking(move || {
        let db_path = std::path::PathBuf::from(&data_dir).join("anchorcore.db");
        let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| rusqlite::Connection::open(&db_path).unwrap());
        let valid_labels = ["internal","public","sensitive","pii"];
        if !valid_labels.contains(&label.as_str()) {
            return Err((StatusCode::UNPROCESSABLE_ENTITY, Json(serde_json::json!({"detail": format!("label must be one of {:?}", valid_labels)}))));
        }
        conn.execute("INSERT INTO sources (name, connector, config, label, enabled) VALUES (?1, ?2, ?3, ?4, 1)", rusqlite::params![name, connector, config_str, label]).unwrap();
        let id = conn.last_insert_rowid();
        let mut stmt = conn.prepare("SELECT id, name, connector, config, enabled, label, last_synced_at, last_error, error_count, created_at FROM sources WHERE id = ?1").unwrap();
        let v = stmt.query_row([id], |r| source_json(r)).unwrap();
        Ok((StatusCode::CREATED, Json(v)))
    }).await.unwrap();
    result
}
