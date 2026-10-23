//! API contract: projects + runtime settings (black-box).
//!
//! Ports `test_projects.py` (including the B15 scoping DoD, which skipped
//! against Rust under the shared-DB harness — fresh DBs make it
//! deterministic) and the HTTP half of `test_settings.py`.

mod common;

use common::{TestServer, decision_doc};
use reqwest::StatusCode;
use serde_json::{Value, json};

#[tokio::test]
async fn project_crud() {
    let srv = TestServer::start().await;
    let resp = srv.post("/projects", &json!({"name": "payments"})).await;
    assert_eq!(resp.status(), StatusCode::CREATED, "POST /projects");
    let created: Value = resp.json().await.expect("project");
    let pid = created["id"].as_i64().unwrap();

    let fetched: Value = srv.get(&format!("/projects/{pid}")).await.json().await.expect("get");
    assert_eq!(fetched["name"], "payments");

    let patched = srv
        .patch(&format!("/projects/{pid}"), &json!({"name": "payments-core", "is_default": true}))
        .await;
    assert_eq!(patched.status(), StatusCode::OK);
    let patched_body: Value = patched.json().await.expect("patch");
    assert_eq!(patched_body["name"], "payments-core");
    assert_eq!(patched_body["is_default"], true);

    let default: Value = srv.get("/projects/default").await.json().await.expect("default");
    assert_eq!(default["id"], pid);

    let del = srv.delete(&format!("/projects/{pid}")).await;
    assert_eq!(del.status(), StatusCode::OK);
    let listed: Value = srv.get("/projects").await.json().await.expect("list");
    assert_eq!(listed, json!([]));
}

#[tokio::test]
async fn project_rejects_unknown_source() {
    let srv = TestServer::start().await;
    let resp = srv.post("/projects", &json!({"name": "bad", "source_ids": [99999]})).await;
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY, "unknown source must 422");
    let body: Value = resp.json().await.expect("err");
    assert!(body.get("detail").is_some(), "{body}");
}

#[tokio::test]
async fn default_project_is_singleton() {
    let srv = TestServer::start().await;
    let a: Value = srv.post("/projects", &json!({"name": "A"})).await.json().await.expect("a");
    let b: Value = srv.post("/projects", &json!({"name": "B"})).await.json().await.expect("b");
    srv.patch(&format!("/projects/{}", a["id"].as_i64().unwrap()), &json!({"is_default": true})).await;
    srv.patch(&format!("/projects/{}", b["id"].as_i64().unwrap()), &json!({"is_default": true})).await;
    let projects: Value = srv.get("/projects").await.json().await.expect("list");
    let defaults: Vec<&Value> = projects
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| p["is_default"] == true)
        .collect();
    assert_eq!(defaults.len(), 1);
    assert_eq!(defaults[0]["id"], b["id"]);
}

#[tokio::test]
async fn project_scopes_qa_results() {
    let srv = TestServer::start().await;
    let mk = async |folder: &str, name: &str| -> Value {
        let doc = format!("# {name}\nDecision: the {name} team ships the ACME movement rules.\n");
        let source = srv.mk_folder_source(folder, &[("doc.md", &doc)], None).await;
        let job = srv.sync_and_wait(source["id"].as_i64().unwrap(), "sync").await;
        assert_eq!(job["status"], "done", "{job}");
        source
    };
    let alpha = mk("alpha", "Alpha").await;
    let beta = mk("beta", "Beta").await;
    let gamma = mk("gamma", "Gamma").await;

    // P1 = {alpha, beta}; P2 = {beta, gamma} (overlap: beta).
    let p1: Value = srv
        .post("/projects", &json!({"name": "P1", "source_ids": [alpha["id"], beta["id"]]}))
        .await
        .json()
        .await
        .expect("p1");
    let p2: Value = srv
        .post("/projects", &json!({"name": "P2", "source_ids": [beta["id"], gamma["id"]]}))
        .await
        .json()
        .await
        .expect("p2");

    let refs = |body: &Value| -> String {
        body["citations"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|c| c["source_ref"].as_str())
            .collect::<Vec<_>>()
            .join(" ")
    };

    let unscoped: Value = srv
        .post("/qa", &json!({"question": "ACME movement rules"}))
        .await
        .json()
        .await
        .expect("qa");
    let all = refs(&unscoped);
    let spanned = ["alpha/doc.md", "beta/doc.md", "gamma/doc.md"]
        .iter()
        .filter(|f| all.contains(*f))
        .count();
    assert!(spanned >= 2, "unscoped query must span sources: {all}");

    let scoped1: Value = srv
        .post("/qa", &json!({"question": "ACME movement rules", "project_id": p1["id"]}))
        .await
        .json()
        .await
        .expect("qa p1");
    let refs1 = refs(&scoped1);
    assert!(refs1.contains("alpha/doc.md") && refs1.contains("beta/doc.md"), "P1 must include its sources: {refs1}");
    assert!(!refs1.contains("gamma/doc.md"), "P1 must not include gamma: {refs1}");

    let scoped2: Value = srv
        .post("/qa", &json!({"question": "ACME movement rules", "project_id": p2["id"]}))
        .await
        .json()
        .await
        .expect("qa p2");
    let refs2 = refs(&scoped2);
    assert!(refs2.contains("gamma/doc.md"), "P2 must include gamma: {refs2}");
    assert!(!refs2.contains("alpha/doc.md"), "P2 must not include alpha: {refs2}");
}

#[tokio::test]
async fn project_sources_relationship() {
    let srv = TestServer::start().await;
    let da = decision_doc("One", "Team one ships ACME", "Ann");
    let db = decision_doc("Two", "Team two ships ACME", "Bob");
    let a = srv.mk_folder_source("one", &[("doc.md", &da)], None).await;
    let b = srv.mk_folder_source("two", &[("doc.md", &db)], None).await;
    let p: Value = srv
        .post("/projects", &json!({"name": "pair", "source_ids": [a["id"], b["id"]]}))
        .await
        .json()
        .await
        .expect("project");
    let mut got: Vec<i64> = p["source_ids"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_i64())
        .collect();
    got.sort_unstable();
    assert_eq!(got, vec![a["id"].as_i64().unwrap(), b["id"].as_i64().unwrap()]);

    let pid = p["id"].as_i64().unwrap();
    let mut rows: Vec<i64> = srv
        .db()
        .prepare("SELECT source_id FROM project_sources WHERE project_id = ?1")
        .expect("stmt")
        .query_map([pid], |r| r.get(0))
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("rows");
    rows.sort_unstable();
    assert_eq!(rows, got);
}

#[tokio::test]
async fn settings_get_returns_env_defaults() {
    let srv = TestServer::start().await;
    let resp = srv.get("/settings").await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = resp.json().await.expect("settings");
    assert!(body.get("ollama_base_url").is_some(), "{body}");
    assert!(body.get("embed_model").is_some(), "{body}");
    // R45.18: the classifier model settings are deleted (ingest is model-free).
    assert!(body.get("classifier_model").is_none() && body.get("classifier_api_key").is_none(), "{body}");
    assert!(body.get("answer_model").is_some(), "{body}");
    assert!(matches!(body.get("answer_api_key"), Some(Value::String(s)) if s.is_empty() || s == "***set***"), "{body}");
}

#[tokio::test]
async fn settings_put_updates_and_persists() {
    let srv = TestServer::start().await;
    let resp = srv.put("/settings", &json!({"answer_model": "my-test-model-1"})).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.json::<Value>().await.expect("put")["answer_model"], "my-test-model-1");

    let again: Value = srv.get("/settings").await.json().await.expect("get");
    assert_eq!(again["answer_model"], "my-test-model-1");
}

#[tokio::test]
async fn settings_unknown_key_rejected() {
    let srv = TestServer::start().await;
    let resp = srv.put("/settings", &json!({"not_a_setting": "x"})).await;
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn settings_secret_masked_and_clearable() {
    let srv = TestServer::start().await;
    let resp = srv.put("/settings", &json!({"answer_api_key": "[REDACTED]"})).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = resp.json().await.expect("put");
    assert_eq!(body["answer_api_key"], "***set***", "secret must be masked: {body}");
    assert!(!body.to_string().contains("sk-test-secret"), "{body}");

    let fetched: Value = srv.get("/settings").await.json().await.expect("get");
    assert_eq!(fetched["answer_api_key"], "***set***", "{fetched}");

    // Clearing restores the empty default.
    let cleared = srv.put("/settings", &json!({"answer_api_key": null})).await;
    assert_eq!(cleared.status(), StatusCode::OK);
    let cleared_body: Value = cleared.json().await.expect("cleared");
    assert_ne!(cleared_body["answer_api_key"], "***set***", "{cleared_body}");
}

#[tokio::test]
async fn settings_clear_restores_env() {
    let srv = TestServer::start().await;
    srv.put("/settings", &json!({"answer_model": "temp-model"})).await;
    let resp = srv.put("/settings", &json!({"answer_model": null})).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_ne!(
        resp.json::<Value>().await.expect("cleared")["answer_model"],
        "temp-model"
    );
}

#[tokio::test]
async fn test_connection_ollama_unreachable() {
    let srv = TestServer::start().await;
    // Harness points Ollama at localhost:1 (allowed via ANCHOR_ALLOW_TEST_URLS)
    // so the probe must fail gracefully, never hang or 500.
    let resp = srv.post("/settings/test-connection", &json!({"provider": "ollama"})).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = resp.json().await.expect("probe");
    assert_eq!(body["provider"], "ollama", "{body}");
    assert_eq!(body["ok"], false, "{body}");
    assert!(!body["message"].as_str().unwrap_or_default().is_empty(), "{body}");
}

#[tokio::test]
async fn test_connection_answer_requires_key() {
    let srv = TestServer::start().await;
    let resp = srv.post("/settings/test-connection", &json!({"provider": "answer"})).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = resp.json().await.expect("probe");
    assert_eq!(body["provider"], "answer", "{body}");
    assert!(body["ok"].is_boolean(), "{body}");
}

#[tokio::test]
async fn test_connection_unknown_provider_rejected() {
    let srv = TestServer::start().await;
    let resp = srv.post("/settings/test-connection", &json!({"provider": "nope"})).await;
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn system_status_reflects_runtime_settings() {
    let srv = TestServer::start().await;
    srv.put("/settings", &json!({"answer_model": "runtime-status-model"})).await;
    let status: Value = srv.get("/system/status").await.json().await.expect("status");
    assert_eq!(status["answer"]["model"], "runtime-status-model", "{status}");
}

#[tokio::test]
async fn embed_key_masked() {
    let srv = TestServer::start().await;
    srv.put("/settings", &json!({"embed_api_key": "emb-secret"})).await;
    let body: Value = srv.get("/settings").await.json().await.expect("get");
    assert_eq!(body["embed_api_key"], "***set***", "{body}");
    assert!(!body.to_string().contains("emb-secret"), "{body}");
    srv.put("/settings", &json!({"embed_api_key": null})).await;
}

#[tokio::test]
async fn reasoning_effort_setting_persists() {
    let srv = TestServer::start().await;
    srv.put("/settings", &json!({"answer_reasoning_effort": "high"})).await;
    let body: Value = srv.get("/settings").await.json().await.expect("get");
    assert_eq!(body["answer_reasoning_effort"], "high", "{body}");
}
