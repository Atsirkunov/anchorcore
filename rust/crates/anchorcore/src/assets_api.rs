//! `GET /media/:uid` — an asset's bytes, served from the DB blob only (R46.9).
//!
//! Never reads the filesystem, so the source folder can be gone and there is
//! no path to traverse. Mounted at `/media`, not `/assets`: `/assets/*` is
//! the embedded SPA bundle directory and is public for the login page
//! (`auth::is_public_path`), so an asset route there would bypass auth and
//! shadow the UI's own scripts. Same label gate as `/qa/search` and the
//! records listings: under an untrusted answer provider a sensitive/pii
//! source's image is refused with `refusal: "gated_provider"`.
//!
//! R46.10: `?format=json` is the agent form for harnesses that can only
//! read JSON (and what MCP `get_asset` calls): `{asset_id, mime, w, h,
//! size, data_base64}`, capped at `AGENT_MAX_BYTES` — over it, 413 with the
//! size and the raw `url`, so MCP and REST refuse the same images. The raw
//! form stays uncapped (a plain download).

use axum::extract::{Path, Query, State};
use base64::Engine;
use serde::Deserialize;
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use crate::health::AppState;

/// Agent-form cap: one image must fit an MCP tool result (base64 adds a
/// third). Long edges are not capped — model clients downscale themselves.
pub const AGENT_MAX_BYTES: usize = 1536 * 1024;

#[derive(Deserialize)]
pub struct MediaQuery {
    pub format: Option<String>,
}

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

fn agent_json(uid: &str, mime: &str, dims: (Option<i64>, Option<i64>), bytes: &[u8]) -> Response {
    let url = format!("{}/{uid}", crate::assets::MEDIA_ROUTE);
    if bytes.len() > AGENT_MAX_BYTES {
        let body = json!({
            "detail": format!("Image is {} bytes; the agent form caps at {} — download the raw url instead", bytes.len(), AGENT_MAX_BYTES),
            "size": bytes.len(), "cap": AGENT_MAX_BYTES, "url": url,
        });
        return (StatusCode::PAYLOAD_TOO_LARGE, Json(body)).into_response();
    }
    Json(json!({
        "asset_id": uid, "mime": mime, "w": dims.0, "h": dims.1, "size": bytes.len(), "url": url,
        "data_base64": base64::engine::general_purpose::STANDARD.encode(bytes),
    }))
    .into_response()
}

pub async fn media_handler(State(state): State<AppState>, Path(uid): Path<String>, Query(q): Query<MediaQuery>) -> Response {
    let want_json = q.format.as_deref().is_some_and(|f| f.eq_ignore_ascii_case("json"));
    let uid_for_db = uid.clone();
    let trusted = crate::answer::is_answer_trusted(&state.settings);
    let data_dir = state.data_dir.clone();
    let loaded = tokio::task::spawn_blocking(move || {
        let conn = crate::common::open_data_db(&data_dir);
        crate::assets::load_media(&conn, &uid_for_db)
    })
    .await;
    let (mime, label, bytes, dims) = match loaded {
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
        Some(bytes) if want_json => agent_json(&uid, &mime, dims, &bytes),
        Some(bytes) => image_response(&mime, bytes),
        None => error(StatusCode::NOT_FOUND, "Image bytes not stored (over the size cap)"),
    }
}
