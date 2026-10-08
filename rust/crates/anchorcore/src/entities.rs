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
pub struct GateQuery {
    pub gate: Option<String>,
}

/// Phase-1 deterministic gating for the extraction plane.
///
/// Callers whose reads flow toward an agent/model pass `?gate=provider`; the
/// server then withholds sensitive/PII content from untrusted providers and
/// audits the decision. The local UI never passes it: owner management views
/// (lists, review queues) stay complete, because UI-fetched bytes never leave
/// for a provider API — gating them would lock the owner out of their own data.
fn provider_gate_fail(
    conn: &rusqlite::Connection,
    settings: &std::sync::Arc<crate::settings::SettingsService>,
    gate: &GateQuery,
    entity_id: i64,
) -> Option<(StatusCode, Json<Value>)> {
    if gate.gate.as_deref() != Some("provider") {
        return None;
    }
    if crate::answer::is_answer_trusted(settings) {
        return None;
    }
    match crate::answer::gate_entity_content(conn, entity_id) {
        Some(reason) => {
            let _ = conn.execute(
                "INSERT INTO system_events (component, level, message, detail) VALUES ('entities','warning',?1,?2)",
                rusqlite::params![format!("entity read gated (provider)"), reason],
            );
            Some((
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({"code":"gated_provider","detail":reason})),
            ))
        }
        None => None,
    }
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
    Ok(serde_json::json!({
        "id": id,
        "item_id": item_id,
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
        "updated_at": updated_at.unwrap_or_default()
    }))
}

pub async fn list_handler(State(state): State<AppState>, Query(q): Query<ListQuery>) -> Json<Value> {
    let data_dir = state.data_dir.clone();
    let limit = q.limit.unwrap_or(500).clamp(1, 2000);
    let kind = q.kind.clone();
    let status = q.status.clone();
    let result = tokio::task::spawn_blocking(move || {
        let conn = crate::common::open_data_db(&data_dir);
        let mut sql = "SELECT id, item_id, kind, summary, reasoning, confidence, author, source_ref, status, owner, window_text, dispute_count, created_at, updated_at FROM entities".to_string();
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
    Query(gate): Query<GateQuery>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let data_dir = state.data_dir.clone();
    let settings = state.settings.clone();
    let result = tokio::task::spawn_blocking(move || {
        let conn = crate::common::open_data_db(&data_dir);
        if let Some(err) = provider_gate_fail(&conn, &settings, &gate, entity_id) {
            return Err(err);
        }
        let mut stmt = conn.prepare("SELECT id, item_id, kind, summary, reasoning, confidence, author, source_ref, status, owner, window_text, dispute_count, created_at, updated_at FROM entities WHERE id = ?1").unwrap();
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
        let mut stmt = conn.prepare("SELECT id, item_id, kind, summary, reasoning, confidence, author, source_ref, status, owner, window_text, dispute_count, created_at, updated_at FROM entities WHERE id = ?1").unwrap();
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
    Query(gate): Query<GateQuery>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let data_dir = state.data_dir.clone();
    let settings = state.settings.clone();
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
        // Phase-1: extraction-plane gate filters related entities (partial sets
        // stay useful; fully-blocked sets refuse deterministically).
        if gate.gate.as_deref() == Some("provider") && !crate::answer::is_answer_trusted(&settings) {
            let before = all.len();
            all.retain(|id| crate::answer::gate_entity_content(&conn, *id).is_none());
            let blocked = before - all.len();
            if blocked > 0 {
                let _ = conn.execute(
                    "INSERT INTO system_events (component, level, message, detail) VALUES ('entities','warning',?1,?2)",
                    rusqlite::params![
                        format!("related read gated (provider)"),
                        format!("entity {}: {} of {} related withheld", entity_id, blocked, before)
                    ],
                );
                if all.is_empty() {
                    return Err((
                        StatusCode::FORBIDDEN,
                        Json(serde_json::json!({"code":"gated_provider","detail":format!("all {} related entities withheld: sensitive/PII, untrusted answer provider", before)})),
                    ));
                }
            }
        }
        let sql = format!("SELECT id, item_id, kind, summary, reasoning, confidence, author, source_ref, status, owner, window_text, dispute_count, created_at, updated_at FROM entities WHERE id IN ({})", crate::common::placeholders(all.len()));
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
        let mut stmt = conn.prepare("SELECT id, item_id, kind, summary, reasoning, confidence, author, source_ref, status, owner, window_text, dispute_count, created_at, updated_at FROM entities WHERE id = ?1").unwrap();
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
    Query(gate): Query<GateQuery>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let data_dir = state.data_dir.clone();
    let settings = state.settings.clone();
    let result = tokio::task::spawn_blocking(move || {
        let conn = crate::common::open_data_db(&data_dir);
        if let Some(err) = provider_gate_fail(&conn, &settings, &gate, entity_id) {
            return Err(err);
        }
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
    async fn get_gated_provider_returns_403_code() {
        let dir = tempdir().unwrap();
        let state = gate_state(&dir, true);
        let eid = sensitive_eid(&state);
        let gate = Query(GateQuery { gate: Some("provider".to_string()) });
        match get_handler(State(state), Path(eid), gate).await {
            Err((status, Json(v))) => {
                assert_eq!(status, StatusCode::FORBIDDEN);
                assert_eq!(v.get("code").and_then(|c| c.as_str()), Some("gated_provider"));
            }
            Ok(_) => panic!("sensitive entity must be gated for untrusted provider"),
        }
    }

    #[tokio::test]
    async fn get_without_gate_and_trusted_stay_open() {
        // No gate flag: owner management plane untouched even when untrusted.
        let dir = tempdir().unwrap();
        let state = gate_state(&dir, true);
        let eid = sensitive_eid(&state);
        let open = Query(GateQuery { gate: None });
        let Json(v) = get_handler(State(state), Path(eid), open).await.expect("UI read must stay open");
        assert_eq!(v.get("id").and_then(|i| i.as_i64()), Some(eid));
        // Trusted provider + gate flag: allowed through.
        let dir2 = tempdir().unwrap();
        let state2 = gate_state(&dir2, false);
        let eid2 = sensitive_eid(&state2);
        let gate = Query(GateQuery { gate: Some("provider".to_string()) });
        let Json(v2) = get_handler(State(state2), Path(eid2), gate).await.expect("trusted read must pass");
        assert_eq!(v2.get("id").and_then(|i| i.as_i64()), Some(eid2));
    }
}
