//! API contract: redacted bug-report bundle (R47.0, black-box).
//!
//! `GET /system/report` maps the last retained query to a shareable bundle.
//! These tests pin the redaction schema exactly: any new serialized field
//! fails here first, forcing a deliberate review of what leaves the box.

mod common;

use common::TestServer;
use reqwest::StatusCode;
use serde_json::{Value, json};
use std::collections::HashSet;

const BODY_MARKER: &str = "VORTEXBODY-UNIQUE-STRING";
const TITLE_A: &str = "RedactProbe Doc Alpha";
const TITLE_B: &str = "RedactProbe Doc Beta";

async fn seed_two_sources(srv: &TestServer) -> (Value, Value) {
    let a = srv
        .mk_folder_source(
            "rptsrc",
            &[(
                "alpha.md",
                &format!("# {TITLE_A}\nQUASARFLUX {BODY_MARKER} SHAREDTOKEN.\n"),
            )],
            None,
        )
        .await;
    let b = srv
        .mk_folder_source(
            "rptsrc2",
            &[("beta.md", &format!("# {TITLE_B}\nVORTEXHELM SHAREDTOKEN.\n"))],
            None,
        )
        .await;
    srv.sync_and_wait(a["id"].as_i64().unwrap(), "sync").await;
    srv.sync_and_wait(b["id"].as_i64().unwrap(), "sync").await;
    (a, b)
}

fn keys(v: &Value) -> HashSet<String> {
    v.as_object().unwrap().keys().cloned().collect()
}

#[tokio::test]
async fn report_empty_is_404() {
    let srv = TestServer::start().await;
    let resp = srv.get("/system/report").await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let body: Value = resp.json().await.expect("error json");
    assert!(body["detail"].as_str().unwrap_or_default().contains("recorded"), "{body}");
}

#[tokio::test]
async fn report_after_search_redacted() {
    let srv = TestServer::start().await;
    seed_two_sources(&srv).await;
    let resp = srv.post("/qa/search", &json!({"query": "QUASARFLUX SHAREDTOKEN"})).await;
    assert_eq!(resp.status(), StatusCode::OK);

    let resp = srv.get("/system/report").await;
    assert_eq!(resp.status(), StatusCode::OK);
    let report: Value = resp.json().await.expect("report json");
    let raw = serde_json::to_string(&report).unwrap();

    // Top-level + section shapes are pinned exactly.
    assert_eq!(
        keys(&report),
        ["report_version", "generated_at", "query", "hits", "index", "environment"]
            .into_iter()
            .map(String::from)
            .collect(),
        "{raw}"
    );
    assert_eq!(report["report_version"], 1);
    assert_eq!(
        keys(&report["query"]),
        ["text", "kind", "scope", "asked_at", "retrieval", "mode", "blocked", "refusal", "hits_returned"]
            .into_iter()
            .map(String::from)
            .collect(),
        "{raw}"
    );
    assert_eq!(report["query"]["text"], "QUASARFLUX SHAREDTOKEN");
    assert_eq!(report["query"]["kind"], "search");
    assert_eq!(report["query"]["scope"], "all");
    assert!(report["query"]["retrieval"].is_string(), "{raw}");

    // Hits carry ranks and scores, never text or names.
    let hits = report["hits"].as_array().unwrap();
    assert!(!hits.is_empty(), "expected retained hits: {raw}");
    let hit_keys: HashSet<String> = [
        "item_ref",
        "title_hash",
        "title_len",
        "ext",
        "kind",
        "doc_type",
        "env",
        "score",
        "decay",
        "boosted",
        "pierced",
        "provenance",
    ]
    .into_iter()
    .map(String::from)
    .collect();
    for h in hits {
        assert_eq!(keys(h), hit_keys, "{h}");
        // Two-chunk corpus: BM25 magnitudes can't clear the pierce bar —
        // tiny corpora never bury (nothing to bury under), so nothing
        // pierces either. The mechanism itself is pinned in lib tests.
        assert_eq!(h["pierced"], false, "{h}");
        assert!(h["item_ref"].as_str().unwrap().starts_with("item-"), "{h}");
        assert_eq!(h["title_hash"].as_str().unwrap().len(), 16, "{h}");
        assert!(h["provenance"].as_array().is_some(), "{h}");
        for leg in h["provenance"].as_array().unwrap() {
            assert!(leg["leg"].is_string() && leg["rank"].is_u64(), "{leg}");
        }
    }
    // Both items surface (shared token) under distinct stable pseudonyms.
    let refs: HashSet<String> =
        hits.iter().filter_map(|h| h["item_ref"].as_str().map(String::from)).collect();
    assert_eq!(refs.len(), 2, "both items must surface: {refs:?}");

    // Sources are pseudonymized with counts only.
    let sources = report["index"]["sources"].as_array().unwrap();
    assert_eq!(sources.len(), 2, "{raw}");
    for s in sources {
        assert_eq!(
            keys(s),
            ["source_ref", "kind", "label", "items", "error_count"]
                .into_iter()
                .map(String::from)
                .collect(),
            "{s}"
        );
    }
    assert!(report["index"]["items"].is_i64(), "{raw}");
    assert!(report["environment"]["version"].as_str().unwrap().len() > 0, "{raw}");

    // Redaction negatives: no body text, titles, or source names anywhere.
    assert!(!raw.contains(BODY_MARKER), "body leaked: {raw}");
    assert!(!raw.contains(TITLE_A) && !raw.contains(TITLE_B), "title leaked: {raw}");
    assert!(!raw.contains("rptsrc"), "source name leaked: {raw}");
    assert!(!raw.contains("doc.md"), "filename leaked: {raw}");
}

#[tokio::test]
async fn report_after_ask_keeps_mode() {
    let srv = TestServer::start().await;
    seed_two_sources(&srv).await;
    let set = srv.put("/settings", &json!({"llm_mode": "agent"})).await;
    assert_eq!(set.status(), StatusCode::OK);
    let resp = srv.post("/qa", &json!({"question": "QUASARFLUX SHAREDTOKEN"})).await;
    assert_eq!(resp.status(), StatusCode::OK);

    let report: Value = srv.get_json("/system/report").await;
    assert_eq!(report["query"]["kind"], "ask");
    assert_eq!(report["query"]["mode"], "agent_compose");
    assert_eq!(report["environment"]["llm_mode"], "agent");
    assert!(!report["hits"].as_array().unwrap().is_empty(), "{report}");
}

#[tokio::test]
async fn report_index_selects_previous() {
    let srv = TestServer::start().await;
    seed_two_sources(&srv).await;
    srv.post("/qa/search", &json!({"query": "QUASARFLUX"})).await;
    srv.post("/qa/search", &json!({"query": "VORTEXHELM"})).await;

    let last: Value = srv.get_json("/system/report").await;
    assert_eq!(last["query"]["text"], "VORTEXHELM");
    let prev: Value = srv.get_json("/system/report?index=1").await;
    assert_eq!(prev["query"]["text"], "QUASARFLUX");
    let resp = srv.get("/system/report?index=9").await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
