//! Entities + disputes + context — port of `backend/app/routers/entities.py:1`.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use serde_json::Value;

use crate::health::AppState;

#[derive(Deserialize)]
pub struct ListQuery {
    pub kind: Option<String>,
    pub status: Option<String>,
    pub limit: Option<i64>,
}

#[derive(Deserialize)]
pub struct UpdatePayload {
    pub kind: Option<String>,
    pub status: Option<String>,
    pub owner: Option<String>,
}

#[derive(Deserialize)]
pub struct DisputePayload {
    pub reason: Option<String>,
    pub user: Option<String>,
}

#[derive(Deserialize)]
pub struct ConflictPayload {
    pub other_entity_id: Option<i64>,
    pub note: Option<String>,
}

fn entity_json_from_row(row: &rusqlite::Row) -> rusqlite::Result<Value> {
    let id: i64 = row.get(0)?;
    let item_id: i64 = row.get(1)?;
    let kind: String = row.get(2)?;
    let summary: String = row.get(3)?;
    let reasoning: String = row.get(4)?;
    let confidence: f64 = row.get(5)?;
    let author: String = row.get(6)?;
    let source_ref: String = row.get(7)?;
    let status: String = row.get(8)?;
    let owner: String = row.get(9)?;
    let window_text: String = row.get(10)?;
    let dispute_count: i64 = row.get(11)?;
    let created_at: Option<String> = row.get(12)?;
    let updated_at: Option<String> = row.get(13)?;
    let doc_type: String = row.get(14).unwrap_or_default();
    // R35.4: the item's effective Sensitive state (override or source
    // label), so one read answers the gate without a second round trip.
    let item_sensitive: bool = row.get::<_, i64>(15).map(|v| v != 0).unwrap_or(false);
    // R32.13: the entity's own chunk (first by id; null when none was
    // stored) — entity rows always link to parent item AND chunk.
    let chunk_id: Option<i64> = row.get(16).unwrap_or(None);
    Ok(serde_json::json!({
        "id": id,
        "item_id": item_id,
        "item_sensitive": item_sensitive,
        "chunk_id": chunk_id,
        "kind": kind,
        "summary": summary,
        "reasoning": reasoning,
        "confidence": confidence,
        "author": author,
        "source_ref": source_ref,
        "status": status,
        "owner": owner,
        "window_text": window_text,
        "dispute_count": dispute_count,
        "created_at": created_at.unwrap_or_default(),
        "updated_at": updated_at.unwrap_or_default(),
        "doc_type": doc_type
    }))
}

/// R35.4: the shared entity column list — scalar subqueries resolve the
/// item's doc type, its effective Sensitive state (override or source
/// label — `common::effective_sensitive_sql`, migration 29's rule), and
/// the entity's own chunk.
fn entity_select_list() -> String {
    format!(
        "SELECT id, item_id, kind, summary, reasoning, confidence, author, source_ref, status, owner, window_text, dispute_count, created_at, updated_at, \
         (SELECT doc_type FROM ingested_items WHERE ingested_items.id = entities.item_id), \
         (SELECT CASE WHEN {} THEN 1 ELSE 0 END FROM ingested_items i JOIN sources s ON s.id = i.source_id WHERE i.id = entities.item_id), \
         (SELECT id FROM chunks WHERE entity_id = entities.id ORDER BY id LIMIT 1) FROM entities",
        crate::common::effective_sensitive_sql("i", "s.label")
    )
}

pub async fn list_handler(State(state): State<AppState>, Query(q): Query<ListQuery>) -> Json<Value> {
    let data_dir = state.data_dir.clone();
    let limit = q.limit.unwrap_or(500).clamp(1, 2000);
    let kind = q.kind.clone();
    let status = q.status.clone();
    let result = tokio::task::spawn_blocking(move || {
        let conn = crate::common::open_data_db(&data_dir);
        let mut sql = entity_select_list();
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![];
        let mut clauses = vec![];
        if let Some(k) = kind {
            clauses.push("kind = ?");
            params.push(Box::new(k));
        }
        if let Some(s) = status {
            clauses.push("status = ?");
            params.push(Box::new(s));
        }
        if !clauses.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&clauses.join(" AND "));
        }
        sql.push_str(" ORDER BY created_at DESC LIMIT ?");
        params.push(Box::new(limit));
        let mut stmt = match conn.prepare(&sql) {
            Ok(s) => s,
            Err(_) => return Value::Array(vec![]),
        };
        let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
        let rows = stmt.query_map(param_refs.as_slice(), |r| entity_json_from_row(r)).unwrap();
        let out: Vec<Value> = rows.filter_map(|r| r.ok()).collect();
        Value::Array(out)
    })
    .await
    .unwrap();
    Json(result)
}

pub async fn get_handler(
    State(state): State<AppState>,
    Path(entity_id): Path<i64>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let data_dir = state.data_dir.clone();
    let result = tokio::task::spawn_blocking(move || {
        let conn = crate::common::open_data_db(&data_dir);
        let mut stmt = conn.prepare(&format!("{} WHERE id = ?1", entity_select_list())).unwrap();
        let v = stmt.query_row([entity_id], |r| entity_json_from_row(r));
        match v {
            Ok(j) => Ok(j),
            Err(_) => Err((StatusCode::NOT_FOUND, Json(serde_json::json!({"detail":"Entity not found"})))),
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
    Path(entity_id): Path<i64>,
    Json(payload): Json<UpdatePayload>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let data_dir = state.data_dir.clone();
    let result = tokio::task::spawn_blocking(move || {
        let conn = crate::common::open_data_db(&data_dir);
        crate::common::require_row(&conn, "entities", entity_id, "Entity not found")?;
        if let Some(k) = payload.kind {
            let _ = conn.execute("UPDATE entities SET kind = ?1 WHERE id = ?2", rusqlite::params![k, entity_id]);
        }
        if let Some(s) = payload.status {
            let _ = conn.execute("UPDATE entities SET status = ?1 WHERE id = ?2", rusqlite::params![s, entity_id]);
        }
        if let Some(o) = payload.owner {
            let _ = conn.execute("UPDATE entities SET owner = ?1 WHERE id = ?2", rusqlite::params![o, entity_id]);
        }
        let _ = conn.execute("UPDATE entities SET updated_at = datetime('now') WHERE id = ?1", [entity_id]);
        let mut stmt = conn.prepare(&format!("{} WHERE id = ?1", entity_select_list())).unwrap();
        let v = stmt.query_row([entity_id], |r| entity_json_from_row(r)).unwrap();
        Ok(v)
    })
    .await
    .unwrap();
    match result {
        Ok(v) => Ok(Json(v)),
        Err(e) => Err(e),
    }
}

pub async fn related_handler(
    State(state): State<AppState>,
    Path(entity_id): Path<i64>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let data_dir = state.data_dir.clone();
    let result = tokio::task::spawn_blocking(move || {
        let conn = crate::common::open_data_db(&data_dir);
        crate::common::require_row(&conn, "entities", entity_id, "Entity not found")?;
        let mut stmt = conn.prepare("SELECT to_entity_id FROM relationships WHERE from_entity_id = ?1").unwrap();
        let out_ids: Vec<i64> = stmt.query_map([entity_id], |r| r.get(0)).unwrap().filter_map(|r| r.ok()).collect();
        let mut stmt2 = conn.prepare("SELECT from_entity_id FROM relationships WHERE to_entity_id = ?1").unwrap();
        let in_ids: Vec<i64> = stmt2.query_map([entity_id], |r| r.get(0)).unwrap().filter_map(|r| r.ok()).collect();
        let mut all: Vec<i64> = out_ids;
        all.extend(in_ids);
        if all.is_empty() {
            return Ok(Value::Array(vec![]));
        }
        let sql = format!("{} WHERE id IN ({})", entity_select_list(), crate::common::placeholders(all.len()));
        let mut stmt3 = conn.prepare(&sql).unwrap();
        let rows = stmt3.query_map(rusqlite::params_from_iter(all.iter()), |r| entity_json_from_row(r)).unwrap();
        let out: Vec<Value> = rows.filter_map(|r| r.ok()).collect();
        Ok(Value::Array(out))
    })
    .await
    .unwrap();
    match result {
        Ok(v) => Ok(Json(v)),
        Err(e) => Err(e),
    }
}

pub async fn dispute_handler(
    State(state): State<AppState>,
    Path(entity_id): Path<i64>,
    Json(payload): Json<DisputePayload>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let data_dir = state.data_dir.clone();
    let reason = payload.reason.unwrap_or_default().trim().to_string();
    let user = payload.user.unwrap_or_default().trim().to_string();
    let result = tokio::task::spawn_blocking(move || {
        let conn = crate::common::open_data_db(&data_dir);
        crate::common::require_row(&conn, "entities", entity_id, "Entity not found")?;
        let _ = conn.execute("INSERT INTO disputes (entity_id, reason, user) VALUES (?1, ?2, ?3)", rusqlite::params![entity_id, reason, user]);
        let _ = conn.execute("UPDATE entities SET dispute_count = COALESCE(dispute_count,0)+1, status='disputed', updated_at=datetime('now') WHERE id=?1", [entity_id]);
        let mut stmt = conn.prepare(&format!("{} WHERE id = ?1", entity_select_list())).unwrap();
        let v = stmt.query_row([entity_id], |r| entity_json_from_row(r)).unwrap();
        Ok(v)
    })
    .await
    .unwrap();
    match result {
        Ok(v) => Ok(Json(v)),
        Err(e) => Err(e),
    }
}

/// R27.4: flag that two entities contradict each other (typically a prose
/// decision vs a code-documented one). One directed `conflicts_with` row —
/// every read queries both directions, like `related`. Unlike disputes, both
/// sides stay visible to Q&A; answers warn instead.
pub async fn conflict_create_handler(
    State(state): State<AppState>,
    Path(entity_id): Path<i64>,
    Json(payload): Json<ConflictPayload>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)> {
    let other = match payload.other_entity_id {
        Some(o) => o,
        None => {
            return Err((
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(serde_json::json!({"detail": "other_entity_id required"})),
            ))
        }
    };
    if other == entity_id {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({"detail": "an entity cannot conflict with itself"})),
        ));
    }
    let note: String = payload.note.unwrap_or_default().trim().chars().take(1000).collect();
    let data_dir = state.data_dir.clone();
    let result = tokio::task::spawn_blocking(move || {
        let conn = crate::common::open_data_db(&data_dir);
        crate::common::require_row(&conn, "entities", entity_id, "Entity not found")?;
        crate::common::require_row(&conn, "entities", other, "Entity not found")?;
        let dup: Option<i64> = conn
            .query_row(
                "SELECT id FROM relationships WHERE kind='conflicts_with' AND ((from_entity_id=?1 AND to_entity_id=?2) OR (from_entity_id=?2 AND to_entity_id=?1))",
                rusqlite::params![entity_id, other],
                |r| r.get(0),
            )
            .ok();
        if dup.is_some() {
            return Err((
                StatusCode::CONFLICT,
                Json(serde_json::json!({"detail": "conflict already flagged"})),
            ));
        }
        conn.execute(
            "INSERT INTO relationships (from_entity_id, to_entity_id, kind, confidence, note) VALUES (?1,?2,'conflicts_with',1.0,?3)",
            rusqlite::params![entity_id, other, note],
        )
        .map_err(|e| {
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(serde_json::json!({"detail": e.to_string()})),
            )
        })?;
        let id = conn.last_insert_rowid();
        let created: String = conn
            .query_row("SELECT created_at FROM relationships WHERE id=?1", [id], |r| r.get(0))
            .unwrap_or_default();
        Ok((
            StatusCode::CREATED,
            Json(serde_json::json!({
                "id": id,
                "from_entity_id": entity_id,
                "to_entity_id": other,
                "kind": "conflicts_with",
                "note": note,
                "created_at": created,
            })),
        ))
    })
    .await
    .unwrap();
    result
}

pub async fn conflict_delete_handler(
    State(state): State<AppState>,
    Path((entity_id, other_id)): Path<(i64, i64)>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let data_dir = state.data_dir.clone();
    let result = tokio::task::spawn_blocking(move || {
        let conn = crate::common::open_data_db(&data_dir);
        let n = conn
            .execute(
                "DELETE FROM relationships WHERE kind='conflicts_with' AND ((from_entity_id=?1 AND to_entity_id=?2) OR (from_entity_id=?2 AND to_entity_id=?1))",
                rusqlite::params![entity_id, other_id],
            )
            .unwrap_or(0);
        if n == 0 {
            return Err((
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"detail": "Conflict not found"})),
            ));
        }
        Ok(Json(serde_json::json!({"deleted": true})))
    })
    .await
    .unwrap();
    result
}

pub async fn conflicts_list_handler(
    State(state): State<AppState>,
    Path(entity_id): Path<i64>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let data_dir = state.data_dir.clone();
    let result = tokio::task::spawn_blocking(move || {
        let conn = crate::common::open_data_db(&data_dir);
        crate::common::require_row(&conn, "entities", entity_id, "Entity not found")?;
        let mut stmt = conn.prepare("SELECT CASE WHEN from_entity_id=?1 THEN to_entity_id ELSE from_entity_id END, note, created_at FROM relationships WHERE kind='conflicts_with' AND (from_entity_id=?1 OR to_entity_id=?1) ORDER BY created_at DESC").unwrap();
        let rows = stmt
            .query_map([entity_id], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                ))
            })
            .unwrap();
        let mut out = Vec::new();
        for row in rows.flatten() {
            out.push(serde_json::json!({"other_entity_id": row.0, "note": row.1, "created_at": row.2}));
        }
        Ok(Json(Value::Array(out)))
    })
    .await
    .unwrap();
    result
}

pub async fn disputes_handler(State(state): State<AppState>, Path(entity_id): Path<i64>) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let data_dir = state.data_dir.clone();
    let result = tokio::task::spawn_blocking(move || {
        let conn = crate::common::open_data_db(&data_dir);
        crate::common::require_row(&conn, "entities", entity_id, "Entity not found")?;
        let mut stmt = conn.prepare("SELECT id, entity_id, reason, user, created_at FROM disputes WHERE entity_id = ?1 ORDER BY created_at DESC").unwrap();
        let rows = stmt.query_map([entity_id], |r| {
            Ok(serde_json::json!({
                "id": r.get::<_, i64>(0)?,
                "entity_id": r.get::<_, i64>(1)?,
                "reason": r.get::<_, String>(2)?,
                "user": r.get::<_, String>(3)?,
                "created_at": r.get::<_, Option<String>>(4)?.unwrap_or_default()
            }))
        }).unwrap();
        let out: Vec<Value> = rows.filter_map(|r| r.ok()).collect();
        Ok(Value::Array(out))
    })
    .await
    .unwrap();
    match result {
        Ok(v) => Ok(Json(v)),
        Err(e) => Err(e),
    }
}

fn expand_context(full_text: &str, window_text: &str, window_index: Option<i64>) -> (Vec<String>, Vec<String>) {
    let mut before: Vec<String> = vec![];
    let mut after: Vec<String> = vec![];
    if full_text.is_empty() || window_text.is_empty() {
        return (before, after);
    }
    let chunks = crate::chunking::chunk_document(full_text);
    if chunks.is_empty() {
        return (before, after);
    }
    let needle = window_text.chars().take(160).collect::<String>().trim().to_string();
    let mut idx: Option<usize> = None;
    for (i, c) in chunks.iter().enumerate() {
        if !needle.is_empty() && c.contains(&needle) {
            idx = Some(i);
            break;
        }
    }
    if idx.is_none() {
        if let Some(wi) = window_index {
            let cand = (wi - 1).max(0) as usize;
            if cand < chunks.len() {
                idx = Some(cand);
            }
        }
    }
    if let Some(i) = idx {
        if i > 0 {
            before.push(chunks[i - 1].clone());
        }
        if i + 1 < chunks.len() {
            after.push(chunks[i + 1].clone());
        }
    }
    (before, after)
}

pub async fn context_handler(
    State(state): State<AppState>,
    Path(entity_id): Path<i64>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let data_dir = state.data_dir.clone();
    let result = tokio::task::spawn_blocking(move || {
        let conn = crate::common::open_data_db(&data_dir);
        let mut stmt = conn.prepare("SELECT id, item_id, source_ref, window_text, window_index, summary FROM entities WHERE id = ?1").unwrap();
        let row: Result<(i64,i64,String,String,Option<i64>,String), _> = stmt.query_row([entity_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)));
        let (eid, item_id, source_ref, window_text, window_index, summary) = match row {
            Ok(v) => v,
            Err(_) => return Err((StatusCode::NOT_FOUND, Json(serde_json::json!({"detail":"Entity not found"})))),
        };
        // item
        let item_row: Result<(String, String, i64), _> = conn.query_row("SELECT title, text, source_id FROM ingested_items WHERE id = ?1", [item_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)));
        let (item_title, full_text, source_id) = match item_row {
            Ok(v) => v,
            Err(_) => {
                return Ok(serde_json::json!({
                    "entity_id": eid,
                    "source_name": null,
                    "source_ref": source_ref,
                    "window_text": window_text,
                    "expanded_before": [],
                    "expanded_after": [],
                    "full_text": "",
                    "highlight": summary,
                    "item_title": ""
                }))
            }
        };
        let source_name: Option<String> = conn.query_row("SELECT name FROM sources WHERE id = ?1", [source_id], |r| r.get(0)).ok();
        let (before, after) = expand_context(&full_text, &window_text, window_index);
        Ok(serde_json::json!({
            "entity_id": eid,
            "source_name": source_name,
            "source_ref": source_ref,
            "window_text": window_text,
            "expanded_before": before,
            "expanded_after": after,
            "full_text": full_text.chars().take(16000).collect::<String>(),
            "highlight": summary,
            "item_title": item_title
        }))
    }).await.unwrap();
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

    fn gate_state(dir: &tempfile::TempDir, untrusted: bool) -> AppState {
        let store = crate::secrets::SecretStore::new(dir.path().join("secrets.enc"));
        let settings = Arc::new(crate::settings::SettingsService::new(store));
        settings.set_data_dir(dir.path());
        let db_path = crate::db::resolve_db_path(&dir.path().to_string_lossy().to_string());
        let conn = crate::db::init_db(&db_path).unwrap();
        let base = if untrusted { "https://untrusted.example.invalid/v1" } else { "http://127.0.0.1:11434/v1" };
        settings.set("answer_base_url", base, &conn).unwrap();
        // One sensitive source with one entity carrying window text.
        conn.execute("INSERT INTO sources (connector, name, label) VALUES ('folder','sen','sensitive')", []).unwrap();
        let sid: i64 = conn.query_row("SELECT id FROM sources WHERE name='sen'", [], |r| r.get(0)).unwrap();
        conn.execute("INSERT INTO ingested_items (source_id, content_hash, title, text) VALUES (?1,'h','t','moonvault sealed')", [sid]).unwrap();
        let item: i64 = conn.query_row("SELECT id FROM ingested_items WHERE source_id=?1", [sid], |r| r.get(0)).unwrap();
        conn.execute("INSERT INTO entities (item_id, kind, summary, window_text) VALUES (?1,'decision','moonvault','moonvault sealed text')", [item]).unwrap();
        let embedder = Arc::new(crate::embedder::Embedder::new(settings.clone()));
        AppState {
            data_dir: dir.path().to_string_lossy().to_string(),
            settings,
            jobs: crate::jobs::JobManager::new(),
            csrf_token: String::new(),
            embedder,
        }
    }

    fn sensitive_eid(state: &AppState) -> i64 {
        let db_path = crate::db::resolve_db_path(&state.data_dir);
        let conn = crate::db::init_db(&db_path).unwrap();
        conn.query_row("SELECT id FROM entities WHERE summary='moonvault'", [], |r| r.get(0)).unwrap()
    }

    #[tokio::test]
    async fn entity_json_carries_effective_sensitive_state() {
        let dir = tempdir().unwrap();
        let state = gate_state(&dir, false);
        let eid = sensitive_eid(&state);
        let db_path = crate::db::resolve_db_path(&state.data_dir);
        let conn = crate::db::init_db(&db_path).unwrap();
        conn.execute("UPDATE ingested_items SET doc_type='code' WHERE id=(SELECT item_id FROM entities WHERE id=?1)", [eid]).unwrap();
        drop(conn);
        let Json(v) = get_handler(State(state.clone()), Path(eid)).await.expect("get must succeed");
        assert_eq!(v.get("doc_type").and_then(|d| d.as_str()), Some("code"));
        assert_eq!(v.get("item_sensitive").and_then(|b| b.as_bool()), Some(true), "sensitive label reads effective");
        // Relabel to internal: the effective state flips instantly (never copied).
        let conn2 = crate::db::init_db(&db_path).unwrap();
        conn2.execute("UPDATE sources SET label='internal' WHERE name='sen'", []).unwrap();
        drop(conn2);
        let Json(v2) = get_handler(State(state.clone()), Path(eid)).await.expect("get must succeed");
        assert_eq!(v2.get("item_sensitive").and_then(|b| b.as_bool()), Some(false));
        // The override escalates whatever the label; clearing restores inherit.
        let conn3 = crate::db::init_db(&db_path).unwrap();
        conn3.execute("UPDATE ingested_items SET sensitive_override=1 WHERE id=(SELECT item_id FROM entities WHERE id=?1)", [eid]).unwrap();
        drop(conn3);
        let Json(v3) = get_handler(State(state.clone()), Path(eid)).await.expect("get must succeed");
        assert_eq!(v3.get("item_sensitive").and_then(|b| b.as_bool()), Some(true));
        let conn4 = crate::db::init_db(&db_path).unwrap();
        conn4.execute("UPDATE ingested_items SET sensitive_override=NULL WHERE id=(SELECT item_id FROM entities WHERE id=?1)", [eid]).unwrap();
        drop(conn4);
        let Json(v4) = get_handler(State(state), Path(eid)).await.expect("get must succeed");
        assert_eq!(v4.get("item_sensitive").and_then(|b| b.as_bool()), Some(false));
    }

    #[tokio::test]
    async fn entity_json_links_parent_item_and_chunk() {
        // R32.13: entity rows always link to parent item AND chunk — the
        // chunk link is explicit null until a chunk is stored, never absent.
        let dir = tempdir().unwrap();
        let state = gate_state(&dir, false);
        let eid = sensitive_eid(&state);
        let Json(v) = get_handler(State(state.clone()), Path(eid)).await.expect("get must succeed");
        assert!(v.get("item_id").and_then(|i| i.as_i64()).is_some(), "item link always present");
        assert_eq!(v.get("chunk_id"), Some(&Value::Null), "chunk link null until stored");
        let db_path = crate::db::resolve_db_path(&state.data_dir);
        let conn = crate::db::init_db(&db_path).unwrap();
        conn.execute("INSERT INTO chunks (entity_id, kind, source_ref, content) VALUES (?1,'entity','ref','moonvault')", [eid]).unwrap();
        let cid = conn.last_insert_rowid();
        drop(conn);
        let Json(v2) = get_handler(State(state), Path(eid)).await.expect("get must succeed");
        assert_eq!(v2.get("chunk_id").and_then(|c| c.as_i64()), Some(cid));
    }

    #[tokio::test]
    async fn conflict_flag_list_unflag_round_trip() {
        let dir = tempdir().unwrap();
        let state = gate_state(&dir, false);
        let a = sensitive_eid(&state);
        let db_path = crate::db::resolve_db_path(&state.data_dir);
        let conn = crate::db::init_db(&db_path).unwrap();
        let item: i64 = conn.query_row("SELECT item_id FROM entities WHERE id=?1", [a], |r| r.get(0)).unwrap();
        conn.execute("INSERT INTO entities (item_id, kind, summary) VALUES (?1,'decision','code choice')", [item]).unwrap();
        let b = conn.last_insert_rowid();
        drop(conn);
        // flag a→b
        let payload = ConflictPayload { other_entity_id: Some(b), note: Some("code uses sessions, doc says JWT".to_string()) };
        let (status, Json(v)) = conflict_create_handler(State(state.clone()), Path(a), Json(payload)).await.expect("flag must succeed");
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(v.get("kind").and_then(|k| k.as_str()), Some("conflicts_with"));
        // visible from both directions with the note
        let Json(list_a) = conflicts_list_handler(State(state.clone()), Path(a)).await.expect("list a");
        assert_eq!(list_a.as_array().map(|l| l.len()), Some(1));
        assert_eq!(list_a[0].get("other_entity_id").and_then(|i| i.as_i64()), Some(b));
        assert_eq!(list_a[0].get("note").and_then(|n| n.as_str()), Some("code uses sessions, doc says JWT"));
        let Json(list_b) = conflicts_list_handler(State(state.clone()), Path(b)).await.expect("list b");
        assert_eq!(list_b[0].get("other_entity_id").and_then(|i| i.as_i64()), Some(a));
        // duplicate in either direction → 409
        let dup = ConflictPayload { other_entity_id: Some(a), note: None };
        match conflict_create_handler(State(state.clone()), Path(b), Json(dup)).await {
            Err((s, _)) => assert_eq!(s, StatusCode::CONFLICT),
            Ok(_) => panic!("duplicate conflict must be rejected"),
        }
        // unflag from the other side works (direction-insensitive)
        let Json(del) = conflict_delete_handler(State(state.clone()), Path((b, a))).await.expect("unflag must succeed");
        assert_eq!(del.get("deleted").and_then(|d| d.as_bool()), Some(true));
        let Json(list_empty) = conflicts_list_handler(State(state.clone()), Path(a)).await.expect("list after unflag");
        assert_eq!(list_empty.as_array().map(|l| l.len()), Some(0));
    }

    #[tokio::test]
    async fn conflict_create_validates_input() {
        let dir = tempdir().unwrap();
        let state = gate_state(&dir, false);
        let a = sensitive_eid(&state);
        // self-conflict → 422
        let slf = ConflictPayload { other_entity_id: Some(a), note: None };
        match conflict_create_handler(State(state.clone()), Path(a), Json(slf)).await {
            Err((s, Json(v))) => {
                assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
                assert!(v.get("detail").and_then(|d| d.as_str()).unwrap_or("").contains("itself"));
            }
            Ok(_) => panic!("self-conflict must be rejected"),
        }
        // missing counterpart → 404
        let missing = ConflictPayload { other_entity_id: Some(999999), note: None };
        match conflict_create_handler(State(state.clone()), Path(a), Json(missing)).await {
            Err((s, _)) => assert_eq!(s, StatusCode::NOT_FOUND),
            Ok(_) => panic!("missing counterpart must be rejected"),
        }
        // unflagging nothing → 404
        match conflict_delete_handler(State(state.clone()), Path((a, 999999))).await {
            Err((s, _)) => assert_eq!(s, StatusCode::NOT_FOUND),
            Ok(_) => panic!("unflag with no link must be rejected"),
        }
    }

}
