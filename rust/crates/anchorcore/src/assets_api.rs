//! `GET /media/:uid` — an asset's bytes, served from the DB blob only (R46.9).
//!
//! Never reads the filesystem, so the source folder can be gone and there is
//! no path to traverse. Mounted at `/media`, not `/assets`: `/assets/*` is
//! the embedded SPA bundle directory and is public for the login page
//! (`auth::is_public_path`), so an asset route there would bypass auth and
//! shadow the UI's own scripts. Same label gate as `/qa/search` and the
//! records listings: under an untrusted answer provider a sensitive/pii
//! source's image is refused with `refusal: "gated_provider"`.

use axum::extract::{Path, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use crate::health::AppState;

fn gated_label(label: &str) -> bool {
    label == "sensitive" || label == "pii"
}

fn error(status: StatusCode, detail: &str) -> Response {
    (status, Json(json!({"detail": detail}))).into_response()
}

/// Inert by construction: user SVG may carry script, so the response is
/// sandboxed (CSP) and never sniffed into another type.
fn image_response(mime: &str, bytes: Vec<u8>) -> Response {
    let mime = if mime.starts_with("image/") { mime } else { "application/octet-stream" };
    let mut resp = bytes.into_response();
    let h = resp.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_str(mime).unwrap_or(HeaderValue::from_static("application/octet-stream")));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("private, max-age=3600"));
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    h.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static("default-src 'none'; style-src 'unsafe-inline'; sandbox"));
    h.insert(header::CONTENT_DISPOSITION, HeaderValue::from_static("inline"));
    resp
}

pub async fn media_handler(State(state): State<AppState>, Path(uid): Path<String>) -> Response {
    let trusted = crate::answer::is_answer_trusted(&state.settings);
    let data_dir = state.data_dir.clone();
    let loaded = tokio::task::spawn_blocking(move || {
        let conn = crate::common::open_data_db(&data_dir);
        crate::assets::load_media(&conn, &uid)
    })
    .await;
    let (mime, label, bytes) = match loaded {
        Ok(Ok(Some(media))) => media,
        Ok(Ok(None)) => return error(StatusCode::NOT_FOUND, "Asset not found"),
        _ => return error(StatusCode::INTERNAL_SERVER_ERROR, "asset lookup failed"),
    };
    if !trusted && gated_label(&label) {
        crate::answer::record_gate_event(&state.data_dir, std::slice::from_ref(&label)).await;
        let body = json!({"detail": "Asset withheld from an untrusted answer provider", "refusal": "gated_provider"});
        return (StatusCode::FORBIDDEN, Json(body)).into_response();
    }
    match bytes {
        Some(bytes) => image_response(&mime, bytes),
        None => error(StatusCode::NOT_FOUND, "Image bytes not stored (over the size cap)"),
    }
}
