//! API contract: backup import body limit (R43.15) — a real archive well over
//! axum's 2 MB default request-body cap imports through `POST /backup/import`.

mod common;

use common::TestServer;
use reqwest::StatusCode;
use serde_json::{Value, json};

/// 1×1 PNG header; the tail is padded with incompressible bytes so the
/// exported archive (deflate) stays over 10 MB.
const PNG_HEAD: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00,
    0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4, 0x89,
];
const IMAGE_BYTES: usize = 10 * 1024 * 1024 + 512 * 1024;

/// Deterministic xorshift noise (deflate cannot shrink it).
fn noise(len: usize) -> Vec<u8> {
    let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        })
        .collect()
}

async fn export_archive(srv: &TestServer) -> Vec<u8> {
    let resp = srv.post("/backup/export", &json!({})).await;
    assert_eq!(resp.status(), StatusCode::OK, "POST /backup/export");
    resp.bytes().await.expect("archive bytes").to_vec()
}

async fn import_archive(srv: &TestServer, archive: Vec<u8>) -> reqwest::Response {
    let form = reqwest::multipart::Form::new()
        .text("options", json!({"mode": "merge", "dry_run": false}).to_string())
        .part("file", reqwest::multipart::Part::bytes(archive).file_name("big.anchorcore-backup"));
    srv.post_multipart("/backup/import", form).await
}

#[tokio::test]
async fn ten_megabyte_archive_imports() {
    let srv = TestServer::start().await;
    let source = srv
        .mk_folder_source("big", &[("big.md", "# Big\n\nThe harbour panorama: ![Panorama](pano.png)\n")], None)
        .await;
    let mut image = PNG_HEAD.to_vec();
    image.extend(noise(IMAGE_BYTES - PNG_HEAD.len()));
    std::fs::write(srv.work.path().join("big").join("pano.png"), &image).expect("png");
    let job = srv.sync_and_wait(source["id"].as_i64().expect("id"), "sync").await;
    assert_eq!(job["status"], "done", "{job}");

    let archive = export_archive(&srv).await;
    assert!(archive.len() >= 10 * 1024 * 1024, "archive only {} bytes", archive.len());

    let resp = srv.delete(&format!("/sources/{}", source["id"])).await;
    assert!(resp.status().is_success(), "DELETE source: {}", resp.status());

    let resp = import_archive(&srv, archive).await;
    let status = resp.status();
    let report: Value = resp.json().await.expect("import json");
    assert_eq!(status, StatusCode::OK, "{report}");
    assert_eq!(report["dry_run"], false, "{report}");
    let sources: Value = srv.get_json("/sources").await;
    let names: Vec<&str> = sources
        .as_array()
        .or_else(|| sources["sources"].as_array())
        .expect("sources list")
        .iter()
        .filter_map(|s| s["name"].as_str())
        .collect();
    assert!(names.contains(&"big"), "imported source missing: {sources}");
}
