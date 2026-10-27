//! API contract (R35.4): a per-document Sensitive override
//! (`ingested_items.sensitive_override`, `PATCH /items/:id`) is enforced
//! like a `sensitive` source label toward an untrusted cloud provider, on
//! every path: `/qa/search` (incl. `public_only`, what MCP `search` calls),
//! `/qa` + `/qa/public` (MCP `ask`), `/decisions`, `/actions`, `/media/:uid`,
//! and the embed paths (ingest, startup backfill, backup re-embed job).
//! Trusted (localhost or `ANCHOR_CLOUD_TRUST=1`) flows on non-public paths;
//! public scopes drop marked items whatever the provider (R35.5). Unmarked
//! items are never gated. Relabels flip the effective state instantly (the
//! label is never copied onto items); an override survives them.
//!
//! The untrusted embedder is a local mock reached as `http://0.0.0.0:<port>`:
//! the trust check only blesses `localhost`/`127.0.0.1`/`::1`, so the server
//! treats it as cloud while the bytes stay on the machine.

mod common;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::{TestServer, action_doc, decision_doc};
use reqwest::StatusCode;
use serde_json::{Value, json};

/// TEST-NET-1: unroutable, never localhost → an untrusted cloud answer URL.
const CLOUD_URL: &str = "https://192.0.2.1";
const MARK: &str = "zorblax";

/// 1×1 transparent PNG.
const PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00,
    0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0D, 0x49,
    0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00,
    0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
];

struct Corpus {
    marked: Vec<i64>,
    asset_uid: String,
}

/// Three docs to mark (decision, actions, diagram with an image) and two
/// plain ones, synced. Nothing is marked yet.
async fn corpus(srv: &TestServer, name: &str, label: Option<&str>) -> (i64, Vec<(String, i64)>) {
    let decision = decision_doc("Zorblax vault rotation", "We decided to rotate the zorblax vault keys monthly", "Zara");
    let actions = action_doc("rotate the zorblax vault keys", "Zara");
    let diagram = "# Zorblax diagram\n\nThe zorblax rotation diagram: ![Zorblax rotation](img/rotation.png)\n";
    let plain = decision_doc("Quillfeather release", "We decided to ship the quillfeather release on Friday", "Quinn");
    let plain_actions = action_doc("ship the quillfeather notes", "Quinn");
    let files = [
        ("zorblax-decision.md", decision.as_str()),
        ("zorblax-actions.md", actions.as_str()),
        ("zorblax-diagram.md", diagram),
        ("quillfeather.md", plain.as_str()),
        ("quillfeather-actions.md", plain_actions.as_str()),
    ];
    let source = srv.mk_folder_source(name, &files, label).await;
    let img = srv.work.path().join(name).join("img");
    std::fs::create_dir_all(&img).expect("img dir");
    std::fs::write(img.join("rotation.png"), PNG).expect("png");
    let sid = source["id"].as_i64().unwrap();
    let job = srv.sync_and_wait(sid, "sync").await;
    assert_eq!(job["status"], "done", "{job}");
    let items = srv.get_json(&format!("/items?source_id={sid}")).await;
    let items = items["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|i| (i["path"].as_str().unwrap_or_default().to_string(), i["id"].as_i64().unwrap()))
        .collect();
    (sid, items)
}

/// Mark every `zorblax-*` item Sensitive through the public API.
async fn mark(srv: &TestServer, items: &[(String, i64)]) -> Corpus {
    let mut marked = Vec::new();
    for (path, id) in items.iter().filter(|(p, _)| p.starts_with(MARK)) {
        let resp = srv.patch(&format!("/items/{id}"), &json!({"sensitive_override": true})).await;
        assert_eq!(resp.status(), StatusCode::OK, "mark {path}");
        marked.push(*id);
    }
    assert_eq!(marked.len(), 3, "{items:?}");
    let diagram = items.iter().find(|(p, _)| p.contains("zorblax-diagram")).expect("diagram item").1;
    let asset_uid: String = srv
        .db()
        .query_row("SELECT uid FROM assets WHERE item_id = ?1", [diagram], |r| r.get(0))
        .expect("diagram asset");
    Corpus { marked, asset_uid }
}

async fn set(srv: &TestServer, pairs: &[(&str, &str)]) {
    for (k, v) in pairs {
        let resp = srv.put("/settings", &json!({ *k: *v })).await;
        assert_eq!(resp.status(), StatusCode::OK, "set {k}");
    }
}

async fn post_json(srv: &TestServer, path: &str, body: Value) -> Value {
    let resp = srv.post(path, &body).await;
    assert_eq!(resp.status(), StatusCode::OK, "POST {path}");
    resp.json().await.expect("json")
}

fn hit_items(body: &Value, key: &str) -> Vec<i64> {
    body[key].as_array().expect(key).iter().filter_map(|h| h["item_id"].as_i64()).collect()
}

fn record_items(body: &Value, key: &str) -> Vec<i64> {
    body[key].as_array().expect(key).iter().filter_map(|r| r["provenance"]["item_id"].as_i64()).collect()
}

fn none_marked(ids: &[i64], c: &Corpus) -> bool {
    ids.iter().all(|i| !c.marked.contains(i))
}

fn any_marked(ids: &[i64], c: &Corpus) -> bool {
    ids.iter().any(|i| c.marked.contains(i))
}

/// Every read path withholds marked items and refuses when nothing survives.
async fn assert_reads_gated(srv: &TestServer, c: &Corpus) {
    for public_only in [false, true] {
        let body = post_json(srv, "/qa/search", json!({"query": "zorblax vault rotation", "public_only": public_only})).await;
        assert!(hit_items(&body, "hits").is_empty(), "search public_only={public_only}: {body}");
        assert_eq!(body["refusal"], "gated_provider", "{body}");
        assert!(body["blocked"].as_u64().unwrap_or(0) >= 1, "{body}");
        // Exact mode shares the gate.
        let exact = post_json(srv, "/qa/search", json!({"query": MARK, "mode": "exact", "public_only": public_only})).await;
        assert_eq!(exact["refusal"], "gated_provider", "exact: {exact}");
    }
    for path in ["/qa", "/qa/public"] {
        let body = post_json(srv, path, json!({"question": "How often are the zorblax vault keys rotated?"})).await;
        assert_eq!(body["refusal"], "gated_provider", "{path}: {body}");
        assert!(none_marked(&hit_items(&body, "citations"), c), "{path}: {body}");
    }
    let decisions = srv.get_json("/decisions").await;
    assert!(none_marked(&record_items(&decisions, "decisions"), c), "{decisions}");
    assert!(decisions["blocked"].as_u64().unwrap_or(0) >= 1, "{decisions}");
    assert!(decisions["refusal"].is_null(), "the plain decision survives: {decisions}");
    let only_marked = srv.get_json("/decisions?q=zorblax").await;
    assert_eq!(only_marked["refusal"], "gated_provider", "{only_marked}");
    let actions = srv.get_json("/actions?q=zorblax").await;
    assert_eq!((actions["total"].as_u64(), actions["refusal"].as_str()), (Some(0), Some("gated_provider")), "{actions}");
    assert!(actions["blocked"].as_u64().unwrap_or(0) >= 1, "{actions}");
    let media = srv.get(&format!("/media/{}", c.asset_uid)).await;
    assert_eq!(media.status(), StatusCode::FORBIDDEN);
    assert_eq!(media.json::<Value>().await.unwrap()["refusal"], "gated_provider");
}

/// Unmarked content keeps flowing to the same untrusted provider.
async fn assert_plain_flows(srv: &TestServer) {
    let body = post_json(srv, "/qa/search", json!({"query": "quillfeather release"})).await;
    assert!(!hit_items(&body, "hits").is_empty(), "{body}");
    assert!(body["refusal"].is_null(), "{body}");
    let decisions = srv.get_json("/decisions?q=quillfeather").await;
    assert_eq!(decisions["total"], 1, "{decisions}");
    let actions = srv.get_json("/actions?q=quillfeather").await;
    assert_eq!(actions["total"], 1, "{actions}");
}

/// Every non-public read path serves marked items (trusted provider).
/// Public scopes drop them whatever the provider (R35.5).
async fn assert_reads_flow(srv: &TestServer, c: &Corpus) {
    let body = post_json(srv, "/qa/search", json!({"query": "zorblax vault rotation"})).await;
    assert!(any_marked(&hit_items(&body, "hits"), c), "{body}");
    assert_eq!((body["blocked"].as_u64(), body["refusal"].is_null()), (Some(0), true), "{body}");
    let public = post_json(srv, "/qa/search", json!({"query": "zorblax vault rotation", "public_only": true})).await;
    assert!(none_marked(&hit_items(&public, "hits"), c), "{public}");
    assert_eq!(public["refusal"], "gated_provider", "{public}");
    let asked = post_json(srv, "/qa/public", json!({"question": "How often are the zorblax vault keys rotated?"})).await;
    assert_eq!(asked["refusal"], "gated_provider", "{asked}");
    assert!(none_marked(&hit_items(&asked, "citations"), c), "{asked}");
    let decisions = srv.get_json("/decisions?q=zorblax").await;
    assert!(any_marked(&record_items(&decisions, "decisions"), c), "{decisions}");
    assert_eq!(decisions["blocked"], 0, "{decisions}");
    let actions = srv.get_json("/actions?q=zorblax").await;
    assert!(any_marked(&record_items(&actions, "actions"), c), "{actions}");
    assert_eq!(srv.get(&format!("/media/{}", c.asset_uid)).await.status(), StatusCode::OK);
}

async fn events(srv: &TestServer, component: &str) -> Vec<Value> {
    srv.get_json(&format!("/system/errors?component={component}&limit=200")).await.as_array().cloned().unwrap_or_default()
}

#[tokio::test]
async fn sensitive_override_gates_every_read_path_under_untrusted_cloud() {
    let srv = TestServer::start().await;
    let (_, items) = corpus(&srv, "marks", Some("public")).await;
    set(&srv, &[("answer_base_url", CLOUD_URL), ("answer_api_key", "sk-test"), ("answer_timeout", "3")]).await;

    // Unmarked → flows to the untrusted provider like before.
    let before = post_json(&srv, "/qa/search", json!({"query": "zorblax vault rotation"})).await;
    assert!(!hit_items(&before, "hits").is_empty(), "unmarked flows: {before}");
    assert_eq!(before["blocked"], 0, "{before}");

    let c = mark(&srv, &items).await;
    assert_reads_gated(&srv, &c).await;
    assert_plain_flows(&srv).await;
    // Agent mode composes nothing server-side but shares the gate.
    set(&srv, &[("llm_mode", "agent")]).await;
    let agent = post_json(&srv, "/qa", json!({"question": "How often are the zorblax vault keys rotated?"})).await;
    assert_eq!(agent["refusal"], "gated_provider", "{agent}");

    // Audit: the shared answer-gate event names the item mark.
    let qa = events(&srv, "qa").await;
    assert!(
        qa.iter().any(|e| e["message"].as_str().unwrap_or_default().contains("answer gate")
            && e["detail"].as_str().unwrap_or_default().contains("item_sensitive")),
        "{qa:?}"
    );

    // Clearing the mark lifts the gate.
    for id in &c.marked {
        srv.patch(&format!("/items/{id}"), &json!({"sensitive_override": Value::Null})).await;
    }
    let after = post_json(&srv, "/qa/search", json!({"query": "zorblax vault rotation"})).await;
    assert!(any_marked(&hit_items(&after, "hits"), &c), "{after}");
}

#[tokio::test]
async fn sensitive_override_flows_to_localhost_answer_provider() {
    let srv = TestServer::start().await;
    let (_, items) = corpus(&srv, "local", Some("public")).await;
    // Localhost is always trusted (closed port: generation fails fast).
    set(&srv, &[("answer_base_url", "http://localhost:1/v1")]).await;
    let c = mark(&srv, &items).await;
    assert_reads_flow(&srv, &c).await;
}

#[tokio::test]
async fn sensitive_override_flows_with_cloud_trust_flag() {
    let srv = TestServer::start_with_env(&[("ANCHOR_CLOUD_TRUST", "1")]).await;
    let (_, items) = corpus(&srv, "trusted", Some("public")).await;
    set(&srv, &[("answer_base_url", CLOUD_URL), ("answer_api_key", "sk-test"), ("answer_timeout", "3")]).await;
    let c = mark(&srv, &items).await;
    assert_reads_flow(&srv, &c).await;
}

#[tokio::test]
async fn relabel_flips_effective_state_but_override_survives() {
    // R35.4: the label is never copied onto items — relabelling flips the
    // effective state instantly, and an override outlives the relabel.
    let srv = TestServer::start().await;
    let (sid, items) = corpus(&srv, "relabel", Some("sensitive")).await;
    set(&srv, &[("answer_base_url", CLOUD_URL), ("answer_api_key", "sk-test"), ("answer_timeout", "3")]).await;
    let gated = post_json(&srv, "/qa/search", json!({"query": "zorblax vault rotation"})).await;
    assert_eq!(gated["refusal"], "gated_provider", "sensitive label gates: {gated}");

    // Mark one doc, relabel to internal: siblings flow, the marked one stays out.
    let decision = items.iter().find(|(p, _)| p.contains("zorblax-decision")).expect("decision item").1;
    let marked = srv.patch(&format!("/items/{decision}"), &json!({"sensitive_override": true})).await;
    assert_eq!(marked.status(), StatusCode::OK);
    let relabeled = srv.put(&format!("/sources/{sid}"), &json!({"label": "internal"})).await;
    assert_eq!(relabeled.status(), StatusCode::OK);
    let body = post_json(&srv, "/qa/search", json!({"query": "zorblax vault rotation"})).await;
    let ids = hit_items(&body, "hits");
    assert!(!ids.is_empty(), "unmarked siblings flow after the relabel: {body}");
    assert!(!ids.contains(&decision), "the override survives the relabel: {body}");
}

// ---- embed paths ----------------------------------------------------------

/// OpenAI-compatible mock embedder that records every input it receives.
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
    // 0.0.0.0 reaches the loopback listener but is not a trusted local host.
    let base = format!("http://0.0.0.0:{}", listener.local_addr().unwrap().port());
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (base, seen)
}

fn saw_mark(seen: &Arc<Mutex<Vec<String>>>) -> bool {
    seen.lock().unwrap().iter().any(|s| s.to_lowercase().contains(MARK))
}

/// Chunks still without a vector, for override-marked or unmarked items.
fn pending(srv: &TestServer, marked: bool) -> i64 {
    let pred = if marked { "i.sensitive_override = 1" } else { "COALESCE(i.sensitive_override, 0) = 0" };
    srv.db()
        .query_row(
            &format!(
                "SELECT COUNT(*) FROM chunks c LEFT JOIN entities e ON e.id = c.entity_id \
                 JOIN ingested_items i ON i.id = COALESCE(c.item_id, e.item_id) \
                 WHERE c.embedding IS NULL AND {pred}"
            ),
            [],
            |r| r.get(0),
        )
        .expect("pending count")
}

async fn wait_until(what: &str, mut ok: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(60);
    while !ok() {
        assert!(Instant::now() < end, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn embed_env(base: &str) -> Vec<(&'static str, String)> {
    vec![("ANCHOR_EMBED_BASE_URL", base.to_string()), ("ANCHOR_EMBED_MIN_SIGNAL", "0".to_string())]
}

fn as_env<'a>(v: &'a [(&'static str, String)]) -> Vec<(&'static str, &'a str)> {
    v.iter().map(|(k, val)| (*k, val.as_str())).collect()
}

#[tokio::test]
async fn sensitive_override_skips_untrusted_embedder_on_backfill_and_ingest() {
    // First sync with no reachable embedder: every chunk stays pending.
    let mut srv = TestServer::start().await;
    let (sid, items) = corpus(&srv, "embed", None).await;
    mark(&srv, &items).await;
    assert!(pending(&srv, true) > 0 && pending(&srv, false) > 0);

    // Startup backfill against the untrusted mock: unmarked drains, marked stays.
    let (base, seen) = recording_embedder().await;
    let env = embed_env(&base);
    srv.restart(&as_env(&env)).await;
    wait_until("unmarked chunks embedded by the backfill", || pending(&srv, false) == 0).await;
    assert!(pending(&srv, true) > 0, "marked chunks must stay pending");
    assert!(!saw_mark(&seen), "the untrusted embedder received marked text");

    // Ingest: edit both kinds and re-sync — new marked chunks stay pending.
    let root = srv.work.path().join("embed");
    let decision = decision_doc("Zorblax vault rotation", "We decided to rotate the zorblax vault keys weekly", "Zara");
    std::fs::write(root.join("zorblax-decision.md"), format!("{decision}\nZorblax backup vault follows the same rotation.\n")).unwrap();
    let plain = decision_doc("Quillfeather release", "We decided to ship the quillfeather release on Monday", "Quinn");
    std::fs::write(root.join("quillfeather.md"), plain).unwrap();
    let calls_before = seen.lock().unwrap().len();
    assert_eq!(srv.sync_and_wait(sid, "sync").await["status"], "done");
    wait_until("the edited unmarked doc embedded", || pending(&srv, false) == 0).await;
    assert!(seen.lock().unwrap().len() > calls_before, "unmarked edits still embed");
    assert!(pending(&srv, true) > 0, "marked chunks must stay pending after re-sync");
    assert!(!saw_mark(&seen), "the untrusted embedder received marked text on ingest");
    let audit = events(&srv, "embed").await;
    assert!(audit.iter().any(|e| e["message"].as_str().unwrap_or_default().contains("embed gate")), "{audit:?}");

    // Trusted: the same mock, allowed by ANCHOR_CLOUD_TRUST, drains marked items.
    let mut trusted = env.clone();
    trusted.push(("ANCHOR_CLOUD_TRUST", "1".to_string()));
    srv.restart(&as_env(&trusted)).await;
    wait_until("marked chunks embedded once trusted", || pending(&srv, true) == 0).await;
    assert!(saw_mark(&seen));
}

#[tokio::test]
async fn sensitive_override_skips_untrusted_embedder_on_backup_reembed() {
    // Archive with marked items and no vectors (the default export).
    let origin = TestServer::start().await;
    let (_, items) = corpus(&origin, "archived", None).await;
    mark(&origin, &items).await;
    let resp = origin.post("/backup/export", &json!({})).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let archive = resp.bytes().await.unwrap().to_vec();

    let (base, seen) = recording_embedder().await;
    let env = embed_env(&base);
    let srv = TestServer::start_with_env(&as_env(&env)).await;
    let form = reqwest::multipart::Form::new()
        .text("options", json!({"mode": "merge", "dry_run": false}).to_string())
        .part("file", reqwest::multipart::Part::bytes(archive).file_name("marks.anchorcore-backup"));
    let resp = srv.post_multipart("/backup/import", form).await;
    let report: Value = resp.json().await.unwrap();
    let jobs = report["reembed_job_ids"].as_array().cloned().unwrap_or_default();
    assert!(!jobs.is_empty(), "import queues the re-embed job: {report}");
    for job in jobs {
        let done = srv.wait_job(job.as_i64().unwrap()).await;
        assert_eq!(done["status"], "done", "{done}");
    }
    wait_until("unmarked chunks re-embedded", || pending(&srv, false) == 0).await;
    assert!(pending(&srv, true) > 0, "marked chunks must stay pending");
    assert!(!saw_mark(&seen), "the untrusted embedder received marked text on re-embed");
    let audit = events(&srv, "embed").await;
    assert!(audit.iter().any(|e| e["message"].as_str().unwrap_or_default().contains("embed gate")), "{audit:?}");
}
