use axum::{extract::State, Json};
use rusqlite::Connection;
use serde_json::{json, Value};
use std::sync::Arc;

use crate::{db, embedder::Embedder, jobs::JobManager, settings::SettingsService};

#[derive(Clone)]
pub struct AppState {
    pub data_dir: String,
    pub settings: Arc<SettingsService>,
    pub jobs: Arc<JobManager>,
    pub csrf_token: String,
    pub embedder: Arc<Embedder>,
}

pub async fn health(State(state): State<AppState>) -> Json<Value> {
    let (failing_sources, pending_embeddings) = {
        let db_path = crate::db::resolve_db_path(&state.data_dir);
        let conn = db::init_db(&db_path).unwrap_or_else(|_| Connection::open(&db_path).unwrap());
        let failing = failing_sources(&conn);
        let pending: i64 = conn
            .query_row("SELECT COUNT(*) FROM chunks WHERE embedding IS NULL", [], |r| r.get(0))
            .unwrap_or(0);
        (failing, pending)
    };

    // Mirrors backend/app/status.py:19 ollama_reachable + answer_provider via SettingsService
    let readiness = crate::ollama::model_readiness(&state.settings).await;
    let ollama = if readiness.reachable { "ok" } else { "offline" };
    let answer_key = answer_provider(&state.settings);
    // R45.17: the model banner needs to know whether an offline Ollama takes
    // search down with it (local embeds) or is irrelevant (cloud embeds).
    let embed_base = state
        .settings
        .get("embed_base_url", None)
        .or_else(|| state.settings.get("ollama_base_url", None))
        .unwrap_or_else(|| "http://localhost:11434".to_string());
    let embedder_provider = if crate::common::provider_is_local(&embed_base) {
        "local"
    } else {
        "cloud"
    };

    Json(json!({
        "status": "ok",
        "data_dir": "redacted",
        "components": {
            "ollama": ollama,
            "answer_key": answer_key,
            "missing_models": readiness.missing_models,
            "answer_ready": crate::answer::answer_ready(&state.settings, readiness.answer_ready),
            "embedder": { "provider": embedder_provider },
            "pending_embeddings": pending_embeddings,
            "tasks": {},
            "classifier": { "concurrency": 4 },
            "failing_sources": failing_sources
        }
    }))
}

fn answer_provider(settings: &SettingsService) -> Value {
    let base = settings
        .get("answer_base_url", None)
        .unwrap_or_else(|| "https://api.openai.com/v1".to_string());
    let key = settings.get("answer_api_key", None).unwrap_or_default();
    if crate::common::provider_is_local(&base) {
        Value::String("ollama".to_string())
    } else if !key.trim().is_empty() {
        Value::String("configured".to_string())
    } else {
        Value::String("missing".to_string())
    }
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
