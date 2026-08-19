//! Projects — port of `backend/app/routers/projects.py:1`.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::health::AppState;

#[derive(Deserialize)]
pub struct ProjectCreate {
    pub name: String,
    pub source_ids: Option<Vec<i64>>,
}

#[derive(Deserialize)]
pub struct ProjectUpdate {
    pub name: Option<String>,
    pub is_default: Option<bool>,
    pub source_ids: Option<Vec<i64>>,
}

fn project_json(conn: &rusqlite::Connection, project_id: i64) -> Option<Value> {
    let (id, name, is_default, created_at): (i64, String, bool, Option<String>) = conn
        .query_row(
            "SELECT id, name, is_default, created_at FROM projects WHERE id = ?1",
            [project_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, i64>(2)? != 0, r.get(3)?)),
        )
        .ok()?;
    let mut stmt = conn.prepare("SELECT source_id FROM project_sources WHERE project_id = ?1 ORDER BY source_id").ok()?;
    let source_ids: Vec<i64> = stmt.query_map([project_id], |r| r.get(0)).unwrap().filter_map(|r| r.ok()).collect();
    Some(serde_json::json!({
        "id": id,
        "name": name,
        "is_default": is_default,
        "created_at": created_at.unwrap_or_default(),
        "source_ids": source_ids
    }))
}

pub async fn list_handler(State(state): State<AppState>) -> Json<Value> {
    let data_dir = state.data_dir.clone();
    let result = tokio::task::spawn_blocking(move || {
        let db_path = std::path::PathBuf::from(&data_dir).join("anchorcore.db");
        let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| rusqlite::Connection::open(&db_path).unwrap());
        let mut stmt = conn.prepare("SELECT id FROM projects ORDER BY created_at").unwrap();
        let ids: Vec<i64> = stmt.query_map([], |r| r.get(0)).unwrap().filter_map(|r| r.ok()).collect();
        let out: Vec<Value> = ids.into_iter().filter_map(|id| project_json(&conn, id)).collect();
        Value::Array(out)
    })
    .await
    .unwrap();
    Json(result)
}

pub async fn create_handler(State(state): State<AppState>, Json(payload): Json<ProjectCreate>) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)> {
    if payload.name.trim().is_empty() {
        return Err((StatusCode::UNPROCESSABLE_ENTITY, Json(serde_json::json!({"detail":"project name is required"}))));
    }
    let data_dir = state.data_dir.clone();
    let name = payload.name.trim().to_string();
    let source_ids = payload.source_ids.unwrap_or_default();
    let result = tokio::task::spawn_blocking(move || {
        let db_path = std::path::PathBuf::from(&data_dir).join("anchorcore.db");
        let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| rusqlite::Connection::open(&db_path).unwrap());
        // validate source_ids before write
        if !source_ids.is_empty() {
            let list = source_ids.iter().map(|id| id.to_string()).collect::<Vec<_>>().join(",");
            let sql = format!("SELECT id FROM sources WHERE id IN ({})", list);
            let mut stmt = conn.prepare(&sql).unwrap();
            let found: Vec<i64> = stmt.query_map([], |r| r.get(0)).unwrap().filter_map(|r| r.ok()).collect();
            if found.len() != source_ids.len() {
                let found_set: std::collections::HashSet<i64> = found.into_iter().collect();
                let missing: Vec<i64> = source_ids.iter().filter(|id| !found_set.contains(id)).cloned().collect();
                return Err((StatusCode::UNPROCESSABLE_ENTITY, Json(serde_json::json!({"detail": format!("unknown source ids: {:?}", missing)}))));
            }
        }
        conn.execute("INSERT INTO projects (name) VALUES (?1)", rusqlite::params![name]).unwrap();
        let pid = conn.last_insert_rowid();
        for sid in &source_ids {
            let _ = conn.execute("INSERT INTO project_sources (project_id, source_id) VALUES (?1, ?2)", rusqlite::params![pid, sid]);
        }
        let j = project_json(&conn, pid).unwrap();
        Ok((StatusCode::CREATED, Json(j)))
    })
    .await
    .unwrap();
    result
}

pub async fn default_handler(State(state): State<AppState>) -> Json<Value> {
    let data_dir = state.data_dir.clone();
    let result = tokio::task::spawn_blocking(move || {
        let db_path = std::path::PathBuf::from(&data_dir).join("anchorcore.db");
        let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| rusqlite::Connection::open(&db_path).unwrap());
        let pid: Option<i64> = conn.query_row("SELECT id FROM projects WHERE is_default = 1 LIMIT 1", [], |r| r.get(0)).ok();
        if let Some(id) = pid {
            project_json(&conn, id).unwrap_or(Value::Null)
        } else {
            Value::Null
        }
    })
    .await
    .unwrap();
    Json(result)
}

pub async fn get_handler(State(state): State<AppState>, Path(project_id): Path<i64>) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let data_dir = state.data_dir.clone();
    let result = tokio::task::spawn_blocking(move || {
        let db_path = std::path::PathBuf::from(&data_dir).join("anchorcore.db");
        let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| rusqlite::Connection::open(&db_path).unwrap());
        match project_json(&conn, project_id) {
            Some(v) => Ok(Json(v)),
            None => Err((StatusCode::NOT_FOUND, Json(serde_json::json!({"detail":"Project not found"})))),
        }
    })
    .await
    .unwrap();
    result
}

pub async fn patch_handler(State(state): State<AppState>, Path(project_id): Path<i64>, Json(payload): Json<ProjectUpdate>) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let data_dir = state.data_dir.clone();
    let result = tokio::task::spawn_blocking(move || {
        let db_path = std::path::PathBuf::from(&data_dir).join("anchorcore.db");
        let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| rusqlite::Connection::open(&db_path).unwrap());
        let exists: bool = conn.query_row("SELECT 1 FROM projects WHERE id = ?1", [project_id], |_| Ok(())).is_ok();
        if !exists {
            return Err((StatusCode::NOT_FOUND, Json(serde_json::json!({"detail":"Project not found"}))));
        }
        if let Some(name) = payload.name {
            if name.trim().is_empty() {
                return Err((StatusCode::UNPROCESSABLE_ENTITY, Json(serde_json::json!({"detail":"project name cannot be empty"}))));
            }
            let _ = conn.execute("UPDATE projects SET name = ?1 WHERE id = ?2", rusqlite::params![name.trim(), project_id]);
        }
        if let Some(is_default) = payload.is_default {
            if is_default {
                let _ = conn.execute("UPDATE projects SET is_default = 0 WHERE id != ?1", [project_id]);
                let _ = conn.execute("UPDATE projects SET is_default = 1 WHERE id = ?1", [project_id]);
            }
        }
        if let Some(sids) = payload.source_ids {
            // validate
            if !sids.is_empty() {
                let list = sids.iter().map(|id| id.to_string()).collect::<Vec<_>>().join(",");
                let sql = format!("SELECT id FROM sources WHERE id IN ({})", list);
                let mut stmt = conn.prepare(&sql).unwrap();
                let found: Vec<i64> = stmt.query_map([], |r| r.get(0)).unwrap().filter_map(|r| r.ok()).collect();
                if found.len() != sids.len() {
                    let found_set: std::collections::HashSet<i64> = found.into_iter().collect();
                    let missing: Vec<i64> = sids.iter().filter(|id| !found_set.contains(id)).cloned().collect();
                    return Err((StatusCode::UNPROCESSABLE_ENTITY, Json(serde_json::json!({"detail": format!("unknown source ids: {:?}", missing)}))));
                }
            }
            let _ = conn.execute("DELETE FROM project_sources WHERE project_id = ?1", [project_id]);
            for sid in sids {
                let _ = conn.execute("INSERT INTO project_sources (project_id, source_id) VALUES (?1, ?2)", rusqlite::params![project_id, sid]);
            }
        }
        Ok(Json(project_json(&conn, project_id).unwrap()))
    })
    .await
    .unwrap();
    result
}

pub async fn delete_handler(State(state): State<AppState>, Path(project_id): Path<i64>) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let data_dir = state.data_dir.clone();
    let result = tokio::task::spawn_blocking(move || {
        let db_path = std::path::PathBuf::from(&data_dir).join("anchorcore.db");
        let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| rusqlite::Connection::open(&db_path).unwrap());
        let exists: bool = conn.query_row("SELECT 1 FROM projects WHERE id = ?1", [project_id], |_| Ok(())).is_ok();
        if !exists {
            return Err((StatusCode::NOT_FOUND, Json(serde_json::json!({"detail":"Project not found"}))));
        }
        let _ = conn.execute("DELETE FROM project_sources WHERE project_id = ?1", [project_id]);
        let _ = conn.execute("DELETE FROM projects WHERE id = ?1", [project_id]);
        Ok(Json(serde_json::json!({"deleted": true})))
    })
    .await
    .unwrap();
    result
}
