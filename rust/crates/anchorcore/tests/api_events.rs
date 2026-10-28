//! API contract: the System tab's errors & warnings panel (R47.2, black-box).
//!
//! Dismiss hides rows without deleting them (the table is also the audit
//! trail), grouping folds repeats, the summary feeds the filter controls,
//! and restore brings dismissed rows back.

mod common;

use common::TestServer;
use reqwest::StatusCode;
use serde_json::{json, Value};

fn seed(srv: &TestServer) {
    let conn = srv.db();
    let rows = [
        ("pipeline", "error", "extract failed", "a.pdf", "2026-10-01 10:00:00"),
        ("pipeline", "error", "extract failed", "b.pdf", "2026-10-02 10:00:00"),
        ("pipeline", "error", "extract failed", "c.pdf", "2026-10-03 10:00:00"),
        ("embedder", "warning", "provider slow", "", "2026-10-04 10:00:00"),
        ("mcp", "info", "mcp tool search", "q", "2026-10-05 10:00:00"),
    ];
    for (component, level, message, detail, at) in rows {
        conn.execute(
            "INSERT INTO system_events (component, level, message, detail, created_at) VALUES (?1,?2,?3,?4,?5)",
            [component, level, message, detail, at],
        )
        .unwrap();
    }
}

fn count_rows(srv: &TestServer) -> i64 {
    srv.db().query_row("SELECT COUNT(*) FROM system_events WHERE component IN ('pipeline','embedder','mcp')", [], |r| r.get(0)).unwrap()
}

#[tokio::test]
async fn grouped_list_folds_repeats_and_filters_levels() {
    let srv = TestServer::start().await;
    seed(&srv);
    let rows = srv.get_json("/system/errors?group=true&levels=error,warning").await;
    let rows = rows.as_array().unwrap();
    let failed = rows.iter().find(|r| r["message"] == "extract failed").expect("grouped row");
    assert_eq!(failed["count"], 3);
    assert_eq!(failed["first_seen"], "2026-10-01 10:00:00");
    assert_eq!(failed["last_seen"], "2026-10-03 10:00:00");
    assert_eq!(failed["detail"], "c.pdf", "newest occurrence's detail");
    assert!(rows.iter().all(|r| r["level"] != "info"), "info filtered out: {rows:?}");

    let resp = srv.get("/system/errors?level=fatal").await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn dismiss_hides_without_deleting_and_restore_brings_back() {
    let srv = TestServer::start().await;
    seed(&srv);
    let before = count_rows(&srv);

    // `source_id: null` = events with no source (the grouped row's own key).
    let resp = srv.post("/system/errors/dismiss", &json!({"component": "pipeline", "level": "error", "source_id": null, "message": "extract failed"})).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["dismissed"], 3);

    let shown = srv.get_json("/system/errors?component=pipeline").await;
    assert!(shown.as_array().unwrap().is_empty(), "dismissed rows hidden by default: {shown}");
    let all = srv.get_json("/system/errors?component=pipeline&include_dismissed=true").await;
    assert_eq!(all.as_array().unwrap().len(), 3);
    assert!(all[0]["dismissed_at"].is_string());
    assert_eq!(count_rows(&srv), before, "dismiss never deletes (audit trail)");

    let summary = srv.get_json("/system/errors/summary").await;
    assert_eq!(summary["levels"]["error"], 0);
    assert_eq!(summary["levels"]["warning"], 1);
    assert_eq!(summary["dismissed"], 3);
    assert!(summary["components"].as_array().unwrap().iter().all(|c| c["component"] != "pipeline"));

    let resp = srv.post("/system/errors/restore", &json!({"component": "pipeline"})).await;
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["restored"], 3);
    assert_eq!(srv.get_json("/system/errors?component=pipeline").await.as_array().unwrap().len(), 3);
}

#[tokio::test]
async fn empty_body_clears_everything_shown_but_a_missing_body_is_refused() {
    let srv = TestServer::start().await;
    seed(&srv);
    let resp = srv.post_empty("/system/errors/dismiss").await;
    assert!(resp.status().is_client_error(), "no body must never mean 'dismiss all': {}", resp.status());
    assert!(!srv.get_json("/system/errors").await.as_array().unwrap().is_empty());

    let body: Value = srv.post("/system/errors/dismiss", &json!({"levels": ["error", "warning"]})).await.json().await.unwrap();
    assert_eq!(body["dismissed"], 4);
    let left = srv.get_json("/system/errors").await;
    assert!(left.as_array().unwrap().iter().all(|r| r["level"] == "info"), "{left}");

    let body: Value = srv.post("/system/errors/dismiss", &json!({})).await.json().await.unwrap();
    assert!(body["dismissed"].as_i64().unwrap() >= 1);
    assert!(srv.get_json("/system/errors").await.as_array().unwrap().is_empty());
}
