//! Projects — port of `backend/app/routers/projects.py:1`.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use serde_json::Value;

use crate::common::{db_err, ApiError};
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
        let db_path = crate::db::resolve_db_path(&data_dir);
        let conn = crate::common::open_db_path(&db_path);
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
    tokio::task::spawn_blocking(move || {
        let conn = crate::common::try_open_data_db(&data_dir).map_err(db_err)?;
        let source_ids = checked_source_ids(&conn, &source_ids)?;
        let tx = conn.unchecked_transaction().map_err(db_err)?;
        tx.execute("INSERT INTO projects (name, uid) VALUES (?1, ?2)", rusqlite::params![name, crate::db::new_uid()]).map_err(db_err)?;
        let pid = tx.last_insert_rowid();
        set_project_sources(&tx, pid, &source_ids)?;
        tx.commit().map_err(db_err)?;
        Ok((StatusCode::CREATED, Json(read_project(&conn, pid)?)))
    })
    .await
    .map_err(db_err)?
}

/// R48.12: requested source ids, deduplicated, all existing — checked before
/// any write (a repeated id used to fail as "unknown source ids: []").
fn checked_source_ids(conn: &rusqlite::Connection, ids: &[i64]) -> Result<Vec<i64>, ApiError> {
    let wanted: Vec<i64> = ids.iter().copied().collect::<std::collections::BTreeSet<_>>().into_iter().collect();
    if wanted.is_empty() {
        return Ok(wanted);
    }
    let sql = format!("SELECT id FROM sources WHERE id IN ({})", crate::common::placeholders(wanted.len()));
    let found: std::collections::HashSet<i64> = conn
        .prepare(&sql)
        .and_then(|mut stmt| stmt.query_map(rusqlite::params_from_iter(wanted.iter()), |r| r.get(0))?.collect())
        .map_err(db_err)?;
    let missing: Vec<i64> = wanted.iter().filter(|id| !found.contains(id)).copied().collect();
    if !missing.is_empty() {
        return Err((StatusCode::UNPROCESSABLE_ENTITY, Json(serde_json::json!({"detail": format!("unknown source ids: {:?}", missing)}))));
    }
    Ok(wanted)
}

fn set_project_sources(tx: &rusqlite::Connection, project_id: i64, source_ids: &[i64]) -> Result<(), ApiError> {
    tx.execute("DELETE FROM project_sources WHERE project_id = ?1", [project_id]).map_err(db_err)?;
    for sid in source_ids {
        tx.execute("INSERT INTO project_sources (project_id, source_id) VALUES (?1, ?2)", rusqlite::params![project_id, sid]).map_err(db_err)?;
    }
    Ok(())
}

fn read_project(conn: &rusqlite::Connection, project_id: i64) -> Result<Value, ApiError> {
    project_json(conn, project_id).ok_or_else(|| (StatusCode::NOT_FOUND, Json(serde_json::json!({"detail":"Project not found"}))))
}

pub async fn default_handler(State(state): State<AppState>) -> Json<Value> {
    let data_dir = state.data_dir.clone();
    let result = tokio::task::spawn_blocking(move || {
        let db_path = crate::db::resolve_db_path(&data_dir);
        let conn = crate::common::open_db_path(&db_path);
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
        let db_path = crate::db::resolve_db_path(&data_dir);
        let conn = crate::common::open_db_path(&db_path);
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
    tokio::task::spawn_blocking(move || patch_project(&data_dir, project_id, payload))
        .await
        .map_err(db_err)?
}

/// R48.12: validate everything first, then one transaction.
fn patch_project(data_dir: &str, project_id: i64, payload: ProjectUpdate) -> Result<Json<Value>, ApiError> {
    let conn = crate::common::try_open_data_db(data_dir).map_err(db_err)?;
    read_project(&conn, project_id)?;
    let name = payload.name.map(|n| n.trim().to_string());
    if name.as_deref() == Some("") {
        return Err((StatusCode::UNPROCESSABLE_ENTITY, Json(serde_json::json!({"detail":"project name cannot be empty"}))));
    }
    let source_ids = payload.source_ids.map(|ids| checked_source_ids(&conn, &ids)).transpose()?;
    let tx = conn.unchecked_transaction().map_err(db_err)?;
    write_project_patch(&tx, project_id, name, payload.is_default == Some(true), source_ids)?;
    tx.commit().map_err(db_err)?;
    Ok(Json(read_project(&conn, project_id)?))
}

/// `make_default: false` is a no-op: some project is always the default once one was picked.
fn write_project_patch(tx: &rusqlite::Connection, project_id: i64, name: Option<String>, make_default: bool, source_ids: Option<Vec<i64>>) -> Result<(), ApiError> {
    if let Some(name) = name {
        tx.execute("UPDATE projects SET name = ?1 WHERE id = ?2", rusqlite::params![name, project_id]).map_err(db_err)?;
    }
    if make_default {
        tx.execute("UPDATE projects SET is_default = (id = ?1)", [project_id]).map_err(db_err)?;
    }
    if let Some(ids) = source_ids {
        set_project_sources(tx, project_id, &ids)?;
    }
    Ok(())
}

pub async fn delete_handler(State(state): State<AppState>, Path(project_id): Path<i64>) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let data_dir = state.data_dir.clone();
    tokio::task::spawn_blocking(move || {
        let conn = crate::common::try_open_data_db(&data_dir).map_err(db_err)?;
        read_project(&conn, project_id)?;
        let tx = conn.unchecked_transaction().map_err(db_err)?;
        tx.execute("DELETE FROM project_sources WHERE project_id = ?1", [project_id]).map_err(db_err)?;
        tx.execute("DELETE FROM projects WHERE id = ?1", [project_id]).map_err(db_err)?;
        tx.commit().map_err(db_err)?;
        Ok(Json(serde_json::json!({"deleted": true})))
    })
    .await
    .map_err(db_err)?
}
