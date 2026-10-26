//! R45.4: `/system/embed-model` — built-in embedder status and the explicit
//! download. The download is a person's action (or the CLI), never a boot
//! side effect; files already in the model directory are verified first, so
//! an offline drop-in needs no network.

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde_json::{json, Value};

use crate::embedder::Provider;
use crate::health::AppState;
use crate::model_store;

/// GET /system/embed-model
pub async fn status_handler(State(state): State<AppState>) -> Json<Value> {
    let embedder = &state.embedder;
    crate::embedder::refresh_ollama_serving(&state.settings).await;
    let dir = embedder.builtin_dir();
    let ready = dir.as_deref().is_some_and(model_store::ready);
    let active = match embedder.active_provider() {
        Provider::Builtin => "builtin",
        Provider::Http => "ollama",
    };
    Json(json!({
        "model": model_store::MODEL_NAME,
        "revision": model_store::REVISION,
        "setting": state.settings.get("embed_provider", None).unwrap_or_else(|| "auto".to_string()),
        "active": active,
        "ready": ready,
        "dir": dir.map(|d| d.to_string_lossy().to_string()),
        "download_bytes": model_store::download_bytes(),
        "backend": crate::embed_local::loaded_backend(),
        "progress": model_store::progress(),
    }))
}

/// POST /system/embed-model/download — 202 started, 409 when one is
/// already running, 200 when the model is ready already.
pub async fn download_handler(State(state): State<AppState>) -> (StatusCode, Json<Value>) {
    let Some(dir) = state.embedder.builtin_dir() else {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"detail": "no data directory for models"})));
    };
    if model_store::ready(&dir) {
        return (StatusCode::OK, Json(json!({"started": false, "ready": true})));
    }
    if !model_store::try_begin() {
        return (StatusCode::CONFLICT, Json(json!({"detail": "the built-in model download is already running"})));
    }
    let pending = state.embedder.chunks_pending.clone();
    tokio::spawn(async move {
        match model_store::prepare(&dir, true).await {
            // Chunks stored while no embedder was reachable get vectors now
            // instead of at the next 15-minute backfill tick.
            Ok(()) => pending.notify_one(),
            Err(e) => tracing::warn!("built-in model download failed: {}", e),
        }
    });
    (StatusCode::ACCEPTED, Json(json!({"started": true, "ready": false})))
}
