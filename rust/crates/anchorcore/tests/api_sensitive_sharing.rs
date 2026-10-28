//! API contract: R35.6 — the person's per-destination opt-in for Sensitive
//! content (Settings → Sensitive sources).
//!
//! Three switches, each off by default: cloud embedder, cloud answer
//! provider, AI agents. Provider switches are bound to the host they named.
//! Only the UI may flip them (`X-Anchor-Client: ui`), each change is
//! audited, and `ANCHOR_CLOUD_TRUST` / `ANCHOR_TRUSTED_PROVIDERS` show as
//! "set by environment". Every R35.5 gated path is checked per switch.

mod common;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::{action_doc, decision_doc, TestServer};
use reqwest::StatusCode;
use serde_json::{json, Value};

const WORD: &str = "glimmerkey";
const RUNBOOK: &str = "# Runbook\n\nIntro line about the glimmerkey cutover.\n\n## Rollback\n\nStep one: drain the glimmerkey queue.\nStep two: revert the routing flag.\n";

/// One synced Sensitive folder source: runbook + decision + action doc.
/// Returns (source id, runbook item id).
async fn sensitive_source(srv: &TestServer, name: &str) -> (i64, i64) {
    let decision = decision_doc(&format!("{WORD} rotation"), &format!("We rotate the {WORD} seals quarterly"), "Hana");
    let action = action_doc(&format!("rotate the {WORD} seals"), "Hana");
    let src = srv
        .mk_folder_source(name, &[("runbook.md", RUNBOOK), ("seal.md", &decision), ("tasks.md", &action)], Some("sensitive"))
        .await;
    let sid = src["id"].as_i64().unwrap();
    assert_eq!(srv.sync_and_wait(sid, "sync").await["status"], "done");
    let item = srv
        .db()
        .query_row("SELECT id FROM ingested_items WHERE source_id = ?1 AND external_id = 'runbook.md'", [sid], |r| r.get(0))
        .expect("runbook item");
    (sid, item)
}

async fn status_of(srv: &TestServer, path: &str) -> StatusCode {
    srv.get(path).await.status()
}

async fn post_json(srv: &TestServer, path: &str, body: Value) -> Value {
    let resp = srv.post(path, &body).await;
    assert_eq!(resp.status(), StatusCode::OK, "POST {path}");
    resp.json().await.expect("json")
}

fn mentions(v: &Value) -> bool {
    v.to_string().to_lowercase().contains(WORD)
}

fn settings_events(srv: &TestServer, level: &str) -> i64 {
    srv.db()
        .query_row("SELECT COUNT(*) FROM system_events WHERE component = 'settings' AND level = ?1 AND message LIKE 'Sensitive sharing%'", [level], |r| r.get(0))
        .unwrap()
}

#[tokio::test]
async fn card_is_closed_by_default_ui_only_and_audited() {
    let srv = TestServer::start().await;
    let card = srv.get_json("/settings/sensitive-sharing").await;
    // Harness answer provider is api.openai.com: cloud, closed.
    assert_eq!(card["answer"]["destination"], "https://api.openai.com", "{card}");
    assert_eq!((card["answer"]["allowed"].as_bool(), card["answer"]["local"].as_bool()), (Some(false), Some(false)), "{card}");
    assert_eq!(card["agents"]["allowed"], false, "{card}");
    assert!(card["agents"]["destination"].as_str().unwrap().contains("AI agents"), "{card}");
    // Harness embedder is a loopback Ollama: local, no switch needed.
    assert_eq!((card["embed"]["local"].as_bool(), card["embed"]["allowed"].as_bool()), (Some(true), Some(true)), "{card}");

    // An agent can never opt itself in.
    let resp = srv.put("/settings/sensitive-sharing", &json!({"agents": true})).await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    // Nor through the generic settings endpoint.
    let resp = srv.put("/settings", &json!({"sensitive_share_agents": "1"})).await;
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
    // A local destination has nothing to allow; unknown keys are refused.
    assert_eq!(srv.put_ui("/settings/sensitive-sharing", &json!({"embed": true})).await.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(srv.put_ui("/settings/sensitive-sharing", &json!({"cloud": true})).await.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(settings_events(&srv, "warning"), 0, "refused changes write nothing");

    let card = srv.share_sensitive(json!({"agents": true, "answer": true})).await;
    assert_eq!((card["agents"]["allowed"].as_bool(), card["answer"]["switch"].as_bool()), (Some(true), Some(true)), "{card}");
    assert_eq!(settings_events(&srv, "warning"), 2, "each switch turned on is audited");
    let card = srv.share_sensitive(json!({"agents": false})).await;
    assert_eq!(card["agents"]["allowed"], false);
    assert_eq!(settings_events(&srv, "info"), 1, "turning off is recorded too");

    // A new answer host closes the answer switch and reports the old one.
    assert_eq!(srv.put("/settings", &json!({"answer_base_url": "https://llm.other.example/v1"})).await.status(), StatusCode::OK);
    let card = srv.get_json("/settings/sensitive-sharing").await;
    assert_eq!(card["answer"]["allowed"], false, "{card}");
    assert_eq!(card["answer"]["stale_for"], "https://api.openai.com", "{card}");
}

#[tokio::test]
async fn environment_overrides_show_as_set_by_environment() {
    let srv = TestServer::start_with_env(&[("ANCHOR_CLOUD_TRUST", "1")]).await;
    let card = srv.get_json("/settings/sensitive-sharing").await;
    for id in ["answer", "agents"] {
        assert_eq!((card[id]["env"].as_bool(), card[id]["allowed"].as_bool(), card[id]["switch"].as_bool()), (Some(true), Some(true), Some(false)), "{id}: {card}");
    }
    let allow = TestServer::start_with_env(&[("ANCHOR_TRUSTED_PROVIDERS", "https://api.openai.com")]).await;
    let card = allow.get_json("/settings/sensitive-sharing").await;
    assert_eq!(card["answer"]["env"], true, "{card}");
    assert_eq!(card["agents"]["allowed"], false, "the provider allowlist never opens agents: {card}");
}

/// Agent-caller reads of every R35.5 surface: (path, opened?) per surface.
async fn agent_reads(srv: &TestServer, item: i64) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    for path in [format!("/items/{item}"), format!("/items/{item}?lines=1-3")] {
        out.push((path.clone(), status_of(srv, &path).await == StatusCode::OK));
    }
    let outline = srv.get_ui_json(&format!("/items/{item}/outline")).await;
    let nid = outline["sections"].as_array().unwrap().iter().find(|s| s["title"] == "Rollback").and_then(|s| s["id"].as_i64()).expect("Rollback");
    for path in [format!("/nodes/{nid}?include=text"), format!("/sections/{nid}/chunks")] {
        out.push((path.clone(), status_of(srv, &path).await == StatusCode::OK));
    }
    let entities = srv.get_ui_json("/entities").await;
    let eid = entities.as_array().unwrap().iter().find(|e| mentions(e)).and_then(|e| e["id"].as_i64()).expect("sensitive entity");
    for path in [format!("/entities/{eid}"), format!("/entities/{eid}/context")] {
        out.push((path.clone(), status_of(srv, &path).await == StatusCode::OK));
    }
    out.push(("/entities".into(), mentions(&srv.get_json("/entities").await)));
    out.push(("/decisions".into(), mentions(&srv.get_json(&format!("/decisions?q={WORD}")).await["decisions"])));
    out.push(("/actions".into(), mentions(&srv.get_json(&format!("/actions?q={WORD}")).await["actions"])));
    let search = post_json(srv, "/qa/search", json!({"query": format!("{WORD} queue")})).await;
    out.push(("/qa/search".into(), mentions(&search["hits"])));
    out
}

#[tokio::test]
async fn agents_switch_opens_every_agent_read_path_and_only_those() {
    let srv = TestServer::start().await;
    let (_, item) = sensitive_source(&srv, "agents-switch").await;

    // Default: every agent read is gated (even the answer switch alone
    // never opens agents — trust is no longer inferred from the provider).
    srv.share_sensitive(json!({"answer": true})).await;
    for (path, open) in agent_reads(&srv, item).await {
        assert!(!open, "{path} must be gated for agents by default");
    }
    srv.put("/settings", &json!({"llm_mode": "agent"})).await;
    let asked = post_json(&srv, "/qa", json!({"question": format!("How often are the {WORD} seals rotated?")})).await;
    assert_eq!(asked["refusal"], "gated_provider", "{asked}");

    // Agents on: every surface opens.
    srv.share_sensitive(json!({"agents": true})).await;
    for (path, open) in agent_reads(&srv, item).await {
        assert!(open, "{path} must open with the agents switch");
    }
    let asked = post_json(&srv, "/qa", json!({"question": format!("How often are the {WORD} seals rotated?")})).await;
    assert_eq!((asked["mode"].as_str(), asked["refusal"].is_null()), (Some("agent_compose"), true), "{asked}");
    assert!(mentions(&asked["citations"]), "{asked}");
    // Public scopes still drop Sensitive whatever the switches say.
    let public = post_json(&srv, "/qa/search", json!({"query": format!("{WORD} queue"), "public_only": true})).await;
    assert!(!mentions(&public["hits"]), "{public}");

    // Off again: gated again.
    srv.share_sensitive(json!({"agents": false})).await;
    for (path, open) in agent_reads(&srv, item).await {
        assert!(!open, "{path} must close when the switch goes off");
    }
}

/// OpenAI-compatible chat mock on `0.0.0.0` (reaches loopback but is not a
/// trusted local host); records every request body.
async fn recording_chat() -> (String, Arc<Mutex<Vec<String>>>) {
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let log = seen.clone();
    let handler = move |axum::Json(body): axum::Json<Value>| {
        let log = log.clone();
        async move {
            log.lock().unwrap().push(body.to_string());
            axum::Json(json!({"choices": [{"index": 0, "message": {"role": "assistant", "content": "Composed from the context."}, "finish_reason": "stop"}]}))
        }
    };
    let app = axum::Router::new().route("/v1/chat/completions", axum::routing::post(handler));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://0.0.0.0:{}/v1", listener.local_addr().unwrap().port());
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (base, seen)
}

fn chat_saw_word(seen: &Arc<Mutex<Vec<String>>>) -> bool {
    seen.lock().unwrap().iter().any(|b| b.to_lowercase().contains(WORD))
}

#[tokio::test]
async fn answer_switch_gates_generation_input_per_caller() {
    let srv = TestServer::start().await;
    let _ = sensitive_source(&srv, "answer-switch").await;
    let (base, seen) = recording_chat().await;
    for (k, v) in [("answer_base_url", base.as_str()), ("answer_api_key", "sk-test"), ("answer_timeout", "5")] {
        assert_eq!(srv.put("/settings", &json!({ k: v })).await.status(), StatusCode::OK);
    }
    // The marker lives in the stored docs only: the caller's own question
    // goes to their answer model by design, the gate covers retrieved context.
    let question = json!({"question": "How do we drain the queue?" });

    // Off: the UI sees citations, but the cloud model never gets the bytes.
    let resp = srv.post_ui("/qa", &question).await;
    let asked: Value = resp.json().await.unwrap();
    assert!(mentions(&asked["citations"]), "UI cites Sensitive: {asked}");
    assert!(!chat_saw_word(&seen), "the cloud answer provider received Sensitive text");

    // On (for this host): generation input carries the Sensitive context.
    srv.share_sensitive(json!({"answer": true})).await;
    let asked: Value = srv.post_ui("/qa", &question).await.json().await.unwrap();
    assert_eq!(asked["mode"], "composed", "{asked}");
    assert!(chat_saw_word(&seen), "the allowed provider gets the Sensitive context");

    // An agent caller without the agents switch: the composed answer would
    // hand the context to the agent, so generation input stays gated too.
    seen.lock().unwrap().clear();
    let asked = post_json(&srv, "/qa", question.clone()).await;
    assert!(!mentions(&asked["citations"]), "{asked}");
    assert!(!chat_saw_word(&seen), "an agent's answer must not be composed from Sensitive text");
}

// ---- embed paths ----------------------------------------------------------

async fn recording_embedder() -> (String, Arc<Mutex<Vec<String>>>) {
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let log = seen.clone();
    let handler = move |axum::Json(body): axum::Json<Value>| {
        let log = log.clone();
        async move {
            let inputs: Vec<String> = match &body["input"] {
                Value::Array(a) => a.iter().map(|v| v.as_str().unwrap_or_default().to_string()).collect(),
                v => vec![v.as_str().unwrap_or_default().to_string()],
            };
            let data: Vec<Value> = (0..inputs.len())
                .map(|i| json!({"index": i, "embedding": [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, (i as f64) / 100.0]}))
                .collect();
            log.lock().unwrap().extend(inputs);
            axum::Json(json!({ "data": data }))
        }
    };
    let app = axum::Router::new().route("/v1/embeddings", axum::routing::post(handler));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://0.0.0.0:{}", listener.local_addr().unwrap().port());
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (base, seen)
}

fn pending_sensitive(srv: &TestServer) -> i64 {
    srv.db()
        .query_row(
            "SELECT COUNT(*) FROM chunks c LEFT JOIN entities e ON e.id = c.entity_id \
             JOIN ingested_items i ON i.id = COALESCE(c.item_id, e.item_id) JOIN sources s ON s.id = i.source_id \
             WHERE c.embedding IS NULL AND s.label = 'sensitive'",
            [],
            |r| r.get(0),
        )
        .unwrap()
}

async fn wait_until(what: &str, mut ok: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(60);
    while !ok() {
        assert!(Instant::now() < end, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test]
async fn embed_switch_opens_ingest_backfill_and_reembed() {
    let (base, seen) = recording_embedder().await;
    let env = [("ANCHOR_EMBED_BASE_URL", base.as_str()), ("ANCHOR_EMBED_MIN_SIGNAL", "0")];
    let srv = TestServer::start_with_env(&env).await;
    let card = srv.get_json("/settings/sensitive-sharing").await;
    assert_eq!((card["embed"]["destination"].as_str(), card["embed"]["allowed"].as_bool()), (Some(base.as_str()), Some(false)), "{card}");

    // Off: ingest and the backfill keep Sensitive chunks from the embedder.
    let (sid, _) = sensitive_source(&srv, "embed-switch").await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(pending_sensitive(&srv) > 0);
    assert!(!seen.lock().unwrap().iter().any(|s| s.to_lowercase().contains(WORD)), "untrusted embedder got Sensitive text");

    // On: the backfill wakes and drains what ingest held back …
    srv.share_sensitive(json!({"embed": true})).await;
    wait_until("Sensitive chunks embedded by the backfill", || pending_sensitive(&srv) == 0).await;
    // … and new ingest embeds directly.
    std::fs::write(srv.work.path().join("embed-switch").join("extra.md"), format!("# Extra\n\nThe {WORD} vault notes.\n")).unwrap();
    assert_eq!(srv.sync_and_wait(sid, "sync").await["status"], "done");
    wait_until("the new Sensitive doc embedded", || pending_sensitive(&srv) == 0).await;

    // Re-embed job (backup import): the switch is not in the archive, so the
    // target machine's own switch decides.
    let resp = srv.post("/backup/export", &json!({})).await;
    let archive = resp.bytes().await.unwrap().to_vec();
    let target = TestServer::start_with_env(&env).await;
    let card = target.share_sensitive(json!({"embed": true})).await;
    assert_eq!(card["embed"]["allowed"], true);
    let form = reqwest::multipart::Form::new()
        .text("options", json!({"mode": "merge", "dry_run": false}).to_string())
        .part("file", reqwest::multipart::Part::bytes(archive).file_name("s.anchorcore-backup"));
    let report: Value = target.post_multipart("/backup/import", form).await.json().await.unwrap();
    for job in report["reembed_job_ids"].as_array().cloned().unwrap_or_default() {
        assert_eq!(target.wait_job(job.as_i64().unwrap()).await["status"], "done");
    }
    wait_until("imported Sensitive chunks re-embedded", || pending_sensitive(&target) == 0).await;
    let imported: i64 = target.db().query_row("SELECT COUNT(*) FROM app_settings WHERE key LIKE 'sensitive_share_%'", [], |r| r.get(0)).unwrap();
    assert_eq!(imported, 1, "only the target's own switch exists — none imported");
}
