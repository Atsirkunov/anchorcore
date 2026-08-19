//! Sources CRUD — minimal port for MCP `list_sources`/`get_source` + QA scoping.
//! Port of `backend/app/routers/sources.py:1` (subset).

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};

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

fn job_json(row: &rusqlite::Row) -> rusqlite::Result<Value> {
    let id: i64 = row.get(0)?;
    let source_id: i64 = row.get(1)?;
    let kind: String = row.get(2)?;
    let status: String = row.get(3)?;
    let total: i64 = row.get(4)?;
    let processed: i64 = row.get(5)?;
    let result_str: String = row.get(6)?;
    let error: Option<String> = row.get(7)?;
    let created_at: Option<String> = row.get(8)?;
    let started_at: Option<String> = row.get(9)?;
    let finished_at: Option<String> = row.get(10)?;
    let result: Value = serde_json::from_str(&result_str).unwrap_or(serde_json::json!({}));
    Ok(serde_json::json!({
        "id": id,
        "source_id": source_id,
        "kind": kind,
        "status": status,
        "total": total,
        "processed": processed,
        "result": result,
        "error": error,
        "created_at": created_at.unwrap_or_default(),
        "started_at": started_at,
        "finished_at": finished_at
    }))
}

pub async fn list_handler(State(state): State<AppState>) -> Json<Value> {
    let data_dir = state.data_dir.clone();
    let result = tokio::task::spawn_blocking(move || {
        let db_path = crate::db::resolve_db_path(&data_dir);
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
        let db_path = crate::db::resolve_db_path(&data_dir);
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
        let db_path = crate::db::resolve_db_path(&data_dir);
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

#[derive(Deserialize)]
pub struct UpdatePayload {
    pub name: Option<String>,
    pub enabled: Option<bool>,
    pub label: Option<String>,
    pub config: Option<Value>,
}

pub async fn update_handler(State(state): State<AppState>, Path(source_id): Path<i64>, Json(payload): Json<UpdatePayload>) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let data_dir = state.data_dir.clone();
    let result = tokio::task::spawn_blocking(move || {
        let db_path = crate::db::resolve_db_path(&data_dir);
        let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| rusqlite::Connection::open(&db_path).unwrap());
        let exists: bool = conn.query_row("SELECT 1 FROM sources WHERE id=?1", [source_id], |_| Ok(())).is_ok();
        if !exists {
            return Err((StatusCode::NOT_FOUND, Json(serde_json::json!({"detail":"Source not found"}))));
        }
        if let Some(name) = payload.name {
            if name.trim().is_empty() {
                return Err((StatusCode::UNPROCESSABLE_ENTITY, Json(serde_json::json!({"detail":"source name cannot be empty"}))));
            }
            let _ = conn.execute("UPDATE sources SET name=?1 WHERE id=?2", rusqlite::params![name.trim(), source_id]);
        }
        if let Some(enabled) = payload.enabled {
            let _ = conn.execute("UPDATE sources SET enabled=?1 WHERE id=?2", rusqlite::params![if enabled {1} else {0}, source_id]);
        }
        if let Some(label) = payload.label {
            let valid = ["internal","public","sensitive","pii"];
            if !valid.contains(&label.as_str()) {
                return Err((StatusCode::UNPROCESSABLE_ENTITY, Json(serde_json::json!({"detail": format!("label must be one of {:?}", valid)}))));
            }
            let _ = conn.execute("UPDATE sources SET label=?1 WHERE id=?2", rusqlite::params![label, source_id]);
        }
        if let Some(cfg) = payload.config {
            let s = serde_json::to_string(&cfg).unwrap();
            let _ = conn.execute("UPDATE sources SET config=?1 WHERE id=?2", rusqlite::params![s, source_id]);
        }
        let mut stmt = conn.prepare("SELECT id, name, connector, config, enabled, label, last_synced_at, last_error, error_count, created_at FROM sources WHERE id=?1").unwrap();
        let v = stmt.query_row([source_id], |r| source_json(r)).unwrap();
        Ok(Json(v))
    }).await.unwrap();
    result
}

pub async fn delete_handler(State(state): State<AppState>, Path(source_id): Path<i64>) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let data_dir = state.data_dir.clone();
    let result = tokio::task::spawn_blocking(move || {
        let db_path = crate::db::resolve_db_path(&data_dir);
        let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| rusqlite::Connection::open(&db_path).unwrap());
        let exists: bool = conn.query_row("SELECT 1 FROM sources WHERE id=?1", [source_id], |_| Ok(())).is_ok();
        if !exists {
            return Err((StatusCode::NOT_FOUND, Json(serde_json::json!({"detail":"Source not found"}))));
        }
        // FK cascade will delete items/entities/chunks/jobs via ON DELETE CASCADE, but we also need to clean project_sources
        let _ = conn.execute("DELETE FROM project_sources WHERE source_id=?1", [source_id]);
        let _ = conn.execute("DELETE FROM sources WHERE id=?1", [source_id]);
        Ok(Json(serde_json::json!({"deleted": true})))
    }).await.unwrap();
    result
}

pub async fn config_handler(State(state): State<AppState>, Path(source_id): Path<i64>) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let data_dir = state.data_dir.clone();
    let result = tokio::task::spawn_blocking(move || {
        let db_path = crate::db::resolve_db_path(&data_dir);
        let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| rusqlite::Connection::open(&db_path).unwrap());
        let config_str: String = match conn.query_row("SELECT config FROM sources WHERE id=?1", [source_id], |r| r.get(0)) {
            Ok(s) => s,
            Err(_) => return Err((StatusCode::NOT_FOUND, Json(serde_json::json!({"detail":"Source not found"})))),
        };
        let mut cfg: Value = serde_json::from_str(&config_str).unwrap_or(json!({}));
        // mask secrets like Python's secrets masking (***set***)
        if let Some(obj) = cfg.as_object_mut() {
            for key in ["token","api_key","password"] {
                if obj.contains_key(key) {
                    obj.insert(key.to_string(), Value::String("***set***".to_string()));
                }
            }
        }
        Ok(Json(cfg))
    }).await.unwrap();
    result
}

pub async fn sync_handler(State(state): State<AppState>, Path(source_id): Path<i64>) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)> {
    sync_inner(state, source_id, false).await
}
pub async fn reclassify_handler(State(state): State<AppState>, Path(source_id): Path<i64>) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)> {
    sync_inner(state, source_id, true).await
}

async fn sync_inner(state: AppState, source_id: i64, force: bool) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)> {
    let data_dir = state.data_dir.clone();
    let jobs = state.jobs.clone();
    let settings = state.settings.clone();
    // create job
    let job_id: i64 = tokio::task::spawn_blocking({
        let data_dir = data_dir.clone();
        let jobs = jobs.clone();
        move || {
            let db_path = crate::db::resolve_db_path(&data_dir);
            let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| rusqlite::Connection::open(&db_path).unwrap());
            let exists: bool = conn.query_row("SELECT 1 FROM sources WHERE id=?1", [source_id], |_| Ok(())).is_ok();
            if !exists {
                return Err((StatusCode::NOT_FOUND, Json(serde_json::json!({"detail":"Source not found"}))));
            }
            let kind = if force { "reclassify" } else { "sync" };
            let id = jobs.create_job(&conn, source_id, kind).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"detail": e.to_string()}))))?;
            // fetch job json to return
            Ok::<i64, (StatusCode, Json<Value>)>(id)
        }
    }).await.unwrap()?;
    // spawn background pipeline
    let data_dir_clone = data_dir.clone();
    let settings_clone = settings.clone();
    let jobs_clone = jobs.clone();
    tokio::spawn(async move {
        // create classifier/embedder on demand
        let classifier = std::sync::Arc::new(crate::classifier::Classifier::new(settings_clone.clone()));
        let embedder = std::sync::Arc::new(crate::embedder::Embedder::new(settings_clone.clone()));
        let pipeline = crate::pipeline::Pipeline::new(classifier, embedder, settings_clone, data_dir_clone.clone());
        pipeline.sync_source(source_id, job_id, force).await;
        // ensure jobs maybe_promote already handled inside pipeline
        let _ = jobs_clone;
    });
    // fetch job to return 202
    let job_val = tokio::task::spawn_blocking(move || {
        let db_path = crate::db::resolve_db_path(&data_dir);
        let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| rusqlite::Connection::open(&db_path).unwrap());
        let mut stmt = conn.prepare("SELECT id, source_id, kind, status, total, processed, result, error, created_at, started_at, finished_at FROM jobs WHERE id=?1").unwrap();
        stmt.query_row([job_id], |r| job_json(r)).unwrap()
    }).await.unwrap();
    Ok((StatusCode::ACCEPTED, Json(job_val)))
}
