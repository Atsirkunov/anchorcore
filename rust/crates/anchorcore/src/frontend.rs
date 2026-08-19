//! Frontend embedding (R5.1) — `include_dir!` embeds `frontend/dist`, serves last.
//! Mirrors `backend/app/main.py` static mount + `packaging.spec` COLLECT.

use axum::{
    body::Body,
    http::{header, StatusCode, Uri},
    response::{IntoResponse, Response},
};
use include_dir::{include_dir, Dir};

static DIST: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../../../frontend/dist");

pub async fn handler(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    // Normalize empty -> index.html
    let candidate = if path.is_empty() { "index.html" } else { path };
    // Try exact file
    if let Some(file) = DIST.get_file(candidate) {
        let mime = mime_guess::from_path(candidate).first_or_octet_stream();
        let body = Body::from(file.contents().to_vec());
        return Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, mime.as_ref())
            .body(body)
            .unwrap();
    }
    // For SPA routes (no file extension), fallback to index.html (client-side routing)
    if !candidate.contains('.') {
        if let Some(index) = DIST.get_file("index.html") {
            let body = Body::from(index.contents().to_vec());
            return Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "text/html")
                .body(body)
                .unwrap();
        }
    }
    // Try with index.html for directory-like paths
    let with_index = format!("{}/index.html", candidate.trim_end_matches('/'));
    if let Some(file) = DIST.get_file(&with_index) {
        let body = Body::from(file.contents().to_vec());
        return Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/html")
            .body(body)
            .unwrap();
    }
    // Not found
    (StatusCode::NOT_FOUND, "Not Found").into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Uri;
    #[tokio::test]
    async fn serves_index() {
        let uri: Uri = "/".parse().unwrap();
        let resp = handler(uri).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let ct = resp.headers().get(header::CONTENT_TYPE).unwrap().to_str().unwrap();
        assert!(ct.contains("text/html"));
    }
    #[tokio::test]
    async fn not_found_for_missing_asset_with_ext() {
        let uri: Uri = "/assets/missing-xyz.js".parse().unwrap();
        let resp = handler(uri).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }
}
