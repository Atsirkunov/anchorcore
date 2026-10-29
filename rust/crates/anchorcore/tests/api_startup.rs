//! R41.7: visible startup — the port is bound before the database is ready.
//! `/health` answers 503 `starting` with phases, other API routes 503, the
//! SPA shell is served; a failed migration keeps serving the error state with
//! the data rolled back. The hooks are debug-build only (`db.rs`).

mod common;

use common::TestServer;
use reqwest::StatusCode;
use serde_json::Value;
use std::time::{Duration, Instant};

async fn health(srv: &TestServer) -> Option<(StatusCode, Value)> {
    let resp = srv.client.get(srv.url("/health")).send().await.ok()?;
    let status = resp.status();
    Some((status, resp.json().await.ok()?))
}

/// Poll `/health` until `pred` holds (or panic after 30 s).
async fn wait_for(srv: &TestServer, what: &str, pred: impl Fn(StatusCode, &Value) -> bool) -> Value {
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        if let Some((status, body)) = health(srv).await {
            if pred(status, &body) {
                return body;
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("/health never reached: {what}");
}

#[tokio::test]
async fn slow_migration_shows_phases_then_turns_healthy() {
    if !cfg!(debug_assertions) {
        return; // test hooks exist only in debug builds
    }
    let srv = TestServer::spawn_unready(&[("ANCHOR_TEST_SLOW_MIGRATION_MS", "150")]);
    let body = wait_for(&srv, "503 starting in migrations", |s, b| {
        s == StatusCode::SERVICE_UNAVAILABLE && b["status"] == "starting" && b["phase"] == "migrations"
    })
    .await;
    // fresh library: every migration + finishing up
    let of = body["of"].as_u64().unwrap();
    assert!(of >= 2, "{body}");
    let step = body["step"].as_u64().unwrap();
    assert!(step >= 1 && step <= of, "{body}");
    assert!(body["detail"].as_str().unwrap().starts_with("Migration "), "{body}");
    assert!(body["started_at"].as_str().is_some_and(|s| !s.is_empty()), "{body}");
    assert!(body["detail_since"].as_str().is_some_and(|s| !s.is_empty()), "{body}");

    // Every other API route is 503 with the plain-words detail.
    for path in ["/sources", "/system/status", "/csrf"] {
        let resp = srv.get(path).await;
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE, "{path}");
        assert!(resp.headers().contains_key("retry-after"), "{path}");
        let b: Value = resp.json().await.unwrap();
        assert_eq!(b["detail"], "AnchorCore is upgrading its database", "{path}");
        assert_eq!(b["status"], "starting");
    }
    let resp = srv.client.post(srv.url("/qa/search")).json(&serde_json::json!({"query": "x"})).send().await.unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    // The SPA shell is served so the person sees the upgrade screen.
    let shell = srv.get("/").await;
    assert_eq!(shell.status(), StatusCode::OK);
    assert!(shell.text().await.unwrap().contains("<div id=\"root\""));

    // Steps advance, then the app turns healthy and the API opens.
    wait_for(&srv, "a later step", |_, b| b["step"].as_u64().is_some_and(|n| n > step) || b["status"] == "ok").await;
    srv.wait_healthy().await;
    let ok = srv.get_json("/health").await;
    assert_eq!(ok["status"], "ok");
    assert_eq!(srv.get("/sources").await.status(), StatusCode::OK);
}

#[tokio::test]
async fn failed_migration_keeps_serving_the_error_with_data_rolled_back() {
    if !cfg!(debug_assertions) {
        return;
    }
    let mut srv = TestServer::spawn_unready(&[("ANCHOR_TEST_FAIL_MIGRATION", "30")]);
    let body = wait_for(&srv, "503 failed", |s, b| s == StatusCode::SERVICE_UNAVAILABLE && b["status"] == "failed").await;
    let error = body["error"].as_str().unwrap();
    assert!(error.contains("migration 30") && error.contains("simulated migration failure"), "{body}");
    let data = body["data_state"].as_str().unwrap();
    assert!(data.contains("Migration 30 was rolled back as one transaction"), "{data}");
    assert!(data.contains("schema version 29"), "{data}");
    assert!(data.contains("versions 1–29") && data.contains("kept"), "{data}");
    assert!(body["log_file"].as_str().unwrap().ends_with("anchorcore.log"), "{body}");
    let support = body["support"].as_str().unwrap();
    assert!(support.contains(env!("CARGO_PKG_VERSION")) && support.contains("simulated migration failure"), "{support}");
    // The home directory never leaks into the public /health body.
    if let Ok(home) = std::env::var("HOME") {
        assert!(!body.to_string().contains(&home), "{body}");
    }

    // Still serving: the shell loads, the API says why it is closed.
    assert_eq!(srv.get("/").await.status(), StatusCode::OK);
    let resp = srv.get("/sources").await;
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    let b: Value = resp.json().await.unwrap();
    assert_eq!(b["status"], "failed");
    assert!(b["detail"].as_str().unwrap().contains("could not upgrade"), "{b}");

    // The data really is at version 29 without migration 30's column.
    {
        let db = srv.db();
        let v: i64 = db.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(v, 29, "migration 30 rolled back, 1–29 kept");
        let has: i64 = db
            .query_row("SELECT COUNT(*) FROM pragma_table_info('system_events') WHERE name = 'dismissed_at'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(has, 0, "migration 30's column was rolled back");
    }

    // The next start resumes from step 30 and finishes.
    srv.restart(&[]).await;
    let v: i64 = srv.db().query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
    assert!(v >= 30, "resumed to the latest version, got {v}");
}

#[tokio::test]
async fn too_little_disk_refuses_before_changing_anything() {
    if !cfg!(debug_assertions) {
        return;
    }
    // Build a library at the latest version, rewind it one migration so the
    // next boot has upgrade work, then fake an almost-full disk.
    let mut srv = TestServer::start().await;
    {
        let db = srv.db();
        db.pragma_update(None, "user_version", 29).unwrap();
    }
    srv.restart_unready(&[("ANCHOR_TEST_FREE_DISK_BYTES", "1")]);
    let body = wait_for(&srv, "503 failed (disk)", |_, b| b["status"] == "failed").await;
    assert!(body["error"].as_str().unwrap().contains("Not enough free disk space"), "{body}");
    assert!(body["data_state"].as_str().unwrap().contains("Nothing in your library was changed"), "{body}");
    let v: i64 = srv.db().query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
    assert_eq!(v, 29, "the refusal ran nothing");
}
