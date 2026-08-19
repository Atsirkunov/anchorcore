use axum::{extract::State, Json};
use rusqlite::Connection;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

use crate::settings::SettingsService;

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Mutex<Connection>>,
    pub data_dir: String,
    pub settings: Arc<SettingsService>,
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

    // Mirrors backend/app/status.py:19 ollama_reachable + answer_provider via SettingsService
    let ollama = if is_ollama_reachable(&state.settings).await { "ok" } else { "offline" };
    let answer_key = answer_provider(&state.settings);

    Json(json!({
        "status": "ok",
        "data_dir": state.data_dir,
        "components": {
            "ollama": ollama,
            "answer_key": answer_key,
            "pending_embeddings": pending_embeddings,
            "tasks": {},
            "classifier": { "concurrency": 4 },
            "failing_sources": failing_sources
        }
    }))
}

async fn is_ollama_reachable(settings: &SettingsService) -> bool {
    // Use SettingsService so DB overrides win, like Python status.py:19
    let base = settings
        .get("ollama_base_url", None)
        .unwrap_or_else(|| "http://localhost:11434".to_string());
    let url = format!("{}/api/tags", base.trim_end_matches('/'));
    let client = match reqwest::Client::builder().timeout(std::time::Duration::from_secs(3)).build() {
        Ok(c) => c,
        Err(_) => return false,
    };
    match client.get(&url).send().await {
        Ok(r) => r.status().is_success(),
        Err(_) => false,
    }
}

fn answer_provider(settings: &SettingsService) -> Value {
    let base = settings
        .get("answer_base_url", None)
        .unwrap_or_else(|| "https://api.openai.com/v1".to_string());
    let key = settings.get("answer_api_key", None).unwrap_or_default();
    if base.starts_with("http://localhost") || base.starts_with("http://127.0.0.1") {
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
