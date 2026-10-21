//! R45.13: the people-owned glossary over HTTP. Replaces the R40.8 "Words
//! to confirm" queue (`/review/vocabulary`): nothing is proposed, so there
//! is nothing to triage — people add entries here or mark a file as a
//! glossary (`PATCH /items/:id {"is_glossary": true}`).
//!
//! `GET /glossary` lists manual + marked-file entries; `POST /glossary`
//! `{term, expansion}` adds a manual entry; `DELETE /glossary/:id` removes a
//! manual one (marked-file rows go away by unmarking the file).

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::health::AppState;

#[derive(Deserialize)]
pub struct GlossaryPayload {
    pub term: String,
    pub expansion: String,
}

type ApiError = (StatusCode, Json<Value>);

fn detail(status: StatusCode, msg: impl Into<String>) -> ApiError {
    (status, Json(json!({"detail": msg.into()})))
}

pub async fn list_handler(State(state): State<AppState>) -> Json<Value> {
    let data_dir = state.data_dir.clone();
    let rows = tokio::task::spawn_blocking(move || {
        let conn = crate::common::open_data_db(&data_dir);
        crate::vocab::list_glossary(&conn)
    })
    .await
    .unwrap_or_default();
    Json(json!(rows))
}

pub async fn create_handler(
    State(state): State<AppState>,
    Json(payload): Json<GlossaryPayload>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let data_dir = state.data_dir.clone();
    let result = tokio::task::spawn_blocking(move || {
        let conn = crate::common::open_data_db(&data_dir);
        crate::vocab::add_manual(&conn, &payload.term, &payload.expansion)
    })
    .await
    .map_err(|e| detail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    match result {
        Ok(entry) => Ok((StatusCode::CREATED, Json(json!(entry)))),
        Err(msg) => Err(detail(StatusCode::UNPROCESSABLE_ENTITY, msg)),
    }
}

pub async fn delete_handler(State(state): State<AppState>, Path(id): Path<i64>) -> Result<Json<Value>, ApiError> {
    let data_dir = state.data_dir.clone();
    let result = tokio::task::spawn_blocking(move || {
        let conn = crate::common::open_data_db(&data_dir);
        crate::vocab::delete_manual(&conn, id)
    })
    .await
    .map_err(|e| detail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    match result {
        Ok(true) => Ok(Json(json!({"deleted": id}))),
        Ok(false) => Err(detail(StatusCode::NOT_FOUND, "Glossary entry not found (file entries go away by unmarking the file)")),
        Err(msg) => Err(detail(StatusCode::INTERNAL_SERVER_ERROR, msg)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tempfile::tempdir;

    fn state(dir: &tempfile::TempDir) -> AppState {
        let data_dir = dir.path().to_string_lossy().to_string();
        let db_path = crate::db::resolve_db_path(&data_dir);
        crate::db::init_db(&db_path).unwrap();
        let store = crate::secrets::SecretStore::new(dir.path().join("secrets.enc"));
        let settings = Arc::new(crate::settings::SettingsService::new(store));
        settings.set_data_dir(dir.path());
        let embedder = Arc::new(crate::embedder::Embedder::new(settings.clone()));
        AppState { data_dir, settings, jobs: crate::jobs::JobManager::new(), csrf_token: String::new(), embedder }
    }

    #[tokio::test]
    async fn glossary_crud_round_trip() {
        let dir = tempdir().unwrap();
        let st = state(&dir);
        let payload = GlossaryPayload { term: "KYC".into(), expansion: "Know your customer".into() };
        let (code, Json(created)) = create_handler(State(st.clone()), Json(payload)).await.unwrap();
        assert_eq!(code, StatusCode::CREATED);
        assert_eq!(created["term"], "kyc");
        let dup = GlossaryPayload { term: "kyc".into(), expansion: "know your customer".into() };
        let err = create_handler(State(st.clone()), Json(dup)).await.unwrap_err();
        assert_eq!(err.0, StatusCode::UNPROCESSABLE_ENTITY);
        let bad = GlossaryPayload { term: "k".into(), expansion: "x".into() };
        assert_eq!(create_handler(State(st.clone()), Json(bad)).await.unwrap_err().0, StatusCode::UNPROCESSABLE_ENTITY);
        let Json(list) = list_handler(State(st.clone())).await;
        assert_eq!(list.as_array().unwrap().len(), 1);
        let id = created["id"].as_i64().unwrap();
        assert!(delete_handler(State(st.clone()), Path(id)).await.is_ok());
        assert_eq!(delete_handler(State(st.clone()), Path(id)).await.unwrap_err().0, StatusCode::NOT_FOUND);
    }
}
