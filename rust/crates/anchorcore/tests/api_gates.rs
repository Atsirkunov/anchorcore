//! API contract: labels, provider trust, public scope, disputes (black-box).
//!
//! Ports `test_gate.py` (HTTP half), `test_b30_gates.py` (the four cloud-gate
//! tests skipped against Rust — the harness sets a cloud answer URL over
//! HTTP instead of monkeypatching Python), `test_disputes.py` reframed for
//! the leaf-pool contract (R33.7: exclusion is observable on the
//! decisions/actions listings), plus the HTTP retrieval spot-checks from
//! `test_planner.py`, `test_retrieval.py`, and `test_followups.py`.

mod common;

use common::{TestServer, action_doc, decision_doc};
use reqwest::StatusCode;
use serde_json::{Value, json};

/// TEST-NET-1: unroutable, never localhost → always an unconfirmed cloud
/// provider without touching the real network beyond a fast connect failure.
const CLOUD_URL: &str = "https://192.0.2.1";

#[tokio::test]
async fn source_label_plumbed_through_api() {
    let srv = TestServer::start().await;
    let doc = decision_doc("Internal note", "A plain internal note", "Ivy");
    let root = srv.work.path().join("label-source");
    std::fs::create_dir_all(&root).expect("corpus");
    std::fs::write(root.join("a.md"), &doc).expect("file");
    let created = srv.post(
        "/sources",
        &json!({"connector": "folder", "name": "label-source", "label": "sensitive",
                "config": {"path": root.to_string_lossy()}}),
    ).await;
    assert_eq!(created.status(), StatusCode::CREATED, "POST /sources");
    let mine: Value = created.json().await.expect("source");
    assert_eq!(mine["label"], "sensitive", "{mine}");
    let sid = mine["id"].as_i64().unwrap();
    // R35.4: `pii` was bundled into `sensitive` — a 400 naming the replacement.
    let legacy = srv.post(
        "/sources",
        &json!({"connector": "folder", "name": "legacy-pii", "label": "pii", "config": {}}),
    ).await;
    assert_eq!(legacy.status(), StatusCode::BAD_REQUEST, "pii label must 400");
    let detail: Value = legacy.json().await.expect("detail");
    assert!(detail["detail"].as_str().unwrap_or_default().contains("sensitive"), "{detail}");
    let legacy_update = srv.put(&format!("/sources/{sid}"), &json!({"label": "pii"})).await;
    assert_eq!(legacy_update.status(), StatusCode::BAD_REQUEST, "pii relabel must 400");

    let updated: Value = srv
        .put(&format!("/sources/{sid}"), &json!({"label": "public"}))
        .await
        .json()
        .await
        .expect("update");
    assert_eq!(updated["label"], "public");

    let bad = srv
        .post("/sources", &json!({"connector": "folder", "name": "bad-label", "label": "secret-ish", "config": {}}))
        .await;
    assert_eq!(bad.status(), StatusCode::UNPROCESSABLE_ENTITY, "invalid label must 422");

    let listed: Value = srv.get("/sources").await.json().await.expect("list");
    assert!(
        listed.as_array().unwrap().iter().any(|s| s["id"] == sid && s["label"] == "public"),
        "{listed}"
    );
}

#[tokio::test]
async fn sensitive_source_still_ingests_with_rules() {
    let srv = TestServer::start().await;
    // Local providers are trusted: a sensitive-labelled source ingests fine.
    let doc = decision_doc(
        "Redact personal details",
        "We decided to redact personal details in the customer report",
        "Rita",
    );
    let source = srv.mk_folder_source("sen-ingest", &[("p.md", &doc)], Some("sensitive")).await;
    let sid = source["id"].as_i64().unwrap();
    let job = srv.sync_and_wait(sid, "sync").await;
    assert_eq!(job["status"], "done", "{job}");

    let item_id: i64 = srv
        .db()
        .query_row("SELECT id FROM ingested_items WHERE source_id = ?1", [sid], |r| r.get(0))
        .expect("item");
    let entities: i64 = srv
        .db()
        .query_row("SELECT COUNT(*) FROM entities WHERE item_id = ?1", [item_id], |r| r.get(0))
        .expect("count");
    assert!(entities >= 1);
}

#[tokio::test]
async fn public_ask_excludes_internal_sources() {
    let srv = TestServer::start().await;
    let pub_doc = decision_doc(
        "Public release date",
        "The public release date is January 5th for everyone",
        "Pat",
    );
    let sec_doc = decision_doc(
        "Internal restructuring",
        "The internal restructuring is confidential to staff only",
        "Sal",
    );
    let pub_src = srv.mk_folder_source("pub", &[("pub.md", &pub_doc)], Some("public")).await;
    let sec_src = srv.mk_folder_source("sec", &[("sec.md", &sec_doc)], Some("internal")).await;
    srv.sync_and_wait(pub_src["id"].as_i64().unwrap(), "sync").await;
    srv.sync_and_wait(sec_src["id"].as_i64().unwrap(), "sync").await;

    let resp = srv.post("/qa/public", &json!({"question": "What was decided about release dates?"})).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = resp.json().await.expect("public qa");
    let refs: Vec<&str> = body["citations"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|c| c["source_ref"].as_str())
        .collect();
    assert!(refs.iter().any(|r| r.contains("pub.md")), "{refs:?}");
    assert!(refs.iter().all(|r| !r.contains("sec.md")), "internal must not appear: {refs:?}");
}

#[tokio::test]
async fn public_ask_needs_a_public_source() {
    let srv = TestServer::start().await;
    let doc = decision_doc("Salary cap", "The salary cap for executives is confidential", "Cfo");
    let src = srv.mk_folder_source("only-internal", &[("only-internal.md", &doc)], Some("internal")).await;
    srv.sync_and_wait(src["id"].as_i64().unwrap(), "sync").await;

    let resp = srv.post("/qa/public", &json!({"question": "What is the salary cap?"})).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = resp.json().await.expect("public qa");
    let refs: Vec<&str> = body["citations"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|c| c["source_ref"].as_str())
        .collect();
    assert!(refs.iter().all(|r| !r.contains("only-internal.md")), "must not leak: {refs:?}");
}

async fn with_cloud_answer(srv: &TestServer) {
    for (k, v) in [("answer_base_url", CLOUD_URL), ("answer_api_key", "sk-test")] {
        let resp = srv.put("/settings", &json!({ k: v })).await;
        assert_eq!(resp.status(), StatusCode::OK, "set {k}");
    }
}

#[tokio::test]
async fn sensitive_source_blocked_from_cloud_answer() {
    let srv = TestServer::start().await;
    let doc = decision_doc(
        "HR complaint details",
        "The HR complaint details are highly sensitive",
        "Hana",
    );
    let sens = srv.mk_folder_source("sens", &[("sens.md", &doc)], Some("sensitive")).await;
    srv.sync_and_wait(sens["id"].as_i64().unwrap(), "sync").await;
    with_cloud_answer(&srv).await;

    let resp = srv.post("/qa", &json!({"question": "What is in the HR complaint?"})).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = resp.json().await.expect("qa");
    // Nothing non-sensitive survives → explicit refusal, no leak.
    assert_eq!(body["refusal"], "gated_provider", "{body}");
    let refs: Vec<&str> = body["citations"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|c| c["source_ref"].as_str())
        .collect();
    assert!(refs.iter().all(|r| !r.contains("sens.md")), "{refs:?}");
}

#[tokio::test]
async fn cloud_trust_flag_allows_sensitive() {
    let srv = TestServer::start_with_env(&[("ANCHOR_CLOUD_TRUST", "1")]).await;
    let doc = decision_doc(
        "Client email",
        "The client email is client@example.com for trusted delivery",
        "Tess",
    );
    let sens = srv.mk_folder_source("trusted-sens", &[("t.md", &doc)], Some("sensitive")).await;
    srv.sync_and_wait(sens["id"].as_i64().unwrap(), "sync").await;
    with_cloud_answer(&srv).await;
    // Bound the (doomed) cloud generation attempt: TEST-NET never answers.
    srv.put("/settings", &json!({"answer_timeout": 3})).await;

    let resp = srv.post("/qa", &json!({"question": "What email was mentioned?"})).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = resp.json().await.expect("qa");
    assert_ne!(body["refusal"], "gated_provider", "{body}");
    assert!(!body["citations"].as_array().unwrap().is_empty(), "trusted provider may cite sensitive: {body}");
}

#[tokio::test]
async fn answer_gate_records_system_event() {
    let srv = TestServer::start().await;
    let doc = decision_doc("CEO phone", "The CEO phone number is 555-123-4567", "Pam");
    let sens = srv.mk_folder_source("audit", &[("audit.md", &doc)], Some("sensitive")).await;
    srv.sync_and_wait(sens["id"].as_i64().unwrap(), "sync").await;
    with_cloud_answer(&srv).await;

    srv.post("/qa", &json!({"question": "What is the CEO phone number?"})).await;
    let events: Value = srv
        .get("/system/errors?component=qa&limit=50")
        .await
        .json()
        .await
        .expect("events");
    assert!(
        events.as_array().unwrap().iter().any(|e| e["message"].as_str().unwrap_or_default().contains("answer gate")),
        "{events}"
    );
}

#[tokio::test]
async fn search_reports_gated_provider() {
    let srv = TestServer::start().await;
    let doc = decision_doc("HR complaint details", "The HR complaint details are highly sensitive", "Hana");
    let sens = srv.mk_folder_source("sens", &[("sens.md", &doc)], Some("sensitive")).await;
    srv.sync_and_wait(sens["id"].as_i64().unwrap(), "sync").await;
    with_cloud_answer(&srv).await;

    let resp = srv.post("/qa/search", &json!({"query": "HR complaint"})).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = resp.json().await.expect("search");
    assert_eq!(body["refusal"], "gated_provider", "{body}");
    assert!(body["blocked"].as_i64().unwrap_or(0) >= 1, "{body}");
}

#[tokio::test]
async fn sensitive_override_roundtrip() {
    // R35.4: the per-document Sensitive override — true sets, null clears
    // (inherit), false is reserved, and the removed `is_pii` key names the
    // new field.
    let srv = TestServer::start().await;
    let doc = decision_doc("Finance contact", "The finance contact is finance@example.com", "Finn");
    let source = srv.mk_folder_source("marked-doc", &[("p.md", &doc)], None).await;
    let sid = source["id"].as_i64().unwrap();
    srv.sync_and_wait(sid, "sync").await;

    let item_id: i64 = srv
        .db()
        .query_row("SELECT id FROM ingested_items WHERE source_id = ?1", [sid], |r| r.get(0))
        .expect("item");
    let patched = srv.patch(&format!("/items/{item_id}"), &json!({"sensitive_override": true})).await;
    assert_eq!(patched.status(), StatusCode::OK);
    assert_eq!(patched.json::<Value>().await.expect("patch")["sensitive_override"], true);

    // R35.5: the marked item is read back as the UI (an agent caller is
    // gated — covered in api_client_gate.rs).
    let fetched: Value = srv.get_ui(&format!("/items/{item_id}")).await.json().await.expect("item");
    assert_eq!(fetched["sensitive_override"], true, "{fetched}");

    let cleared = srv.patch(&format!("/items/{item_id}"), &json!({"sensitive_override": Value::Null})).await;
    assert_eq!(cleared.status(), StatusCode::OK);
    assert!(cleared.json::<Value>().await.expect("clear")["sensitive_override"].is_null());

    let reserved = srv.patch(&format!("/items/{item_id}"), &json!({"sensitive_override": false})).await;
    assert_eq!(reserved.status(), StatusCode::BAD_REQUEST, "false is reserved");
    let legacy = srv.patch(&format!("/items/{item_id}"), &json!({"is_pii": true})).await;
    assert_eq!(legacy.status(), StatusCode::BAD_REQUEST, "removed key must 400");
    let detail: Value = legacy.json().await.expect("detail");
    assert!(detail["detail"].as_str().unwrap_or_default().contains("sensitive_override"), "{detail}");
}

#[tokio::test]
async fn dispute_records_audit_and_increments() {
    let srv = TestServer::start().await;
    let doc = decision_doc("Azure-blue stack", "We decided to adopt the azure-blue stack for checkout", "Azra");
    let source = srv.mk_folder_source("dispute-me", &[("d.md", &doc)], None).await;
    srv.sync_and_wait(source["id"].as_i64().unwrap(), "sync").await;

    let entities: Value = srv.get("/entities").await.json().await.expect("entities");
    let entity = entities
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["summary"].as_str().unwrap_or_default().to_lowercase().contains("azure-blue"))
        .expect("azure-blue entity")
        .clone();
    let eid = entity["id"].as_i64().unwrap();

    let resp = srv
        .post(&format!("/entities/{eid}/dispute"), &json!({"reason": "superseded by the teal decision", "user": "reviewer@corp"}))
        .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let updated: Value = resp.json().await.expect("dispute");
    assert_eq!(updated["status"], "disputed", "{updated}");
    assert_eq!(updated["dispute_count"], 1, "{updated}");

    let history: Value = srv.get(&format!("/entities/{eid}/disputes")).await.json().await.expect("history");
    assert_eq!(history.as_array().unwrap().len(), 1, "{history}");
    assert_eq!(history[0]["reason"], "superseded by the teal decision");
    assert_eq!(history[0]["user"], "reviewer@corp");

    let again = srv.post(&format!("/entities/{eid}/dispute"), &json!({"reason": "double-checked"})).await;
    assert_eq!(again.status(), StatusCode::OK);
    assert_eq!(again.json::<Value>().await.expect("again")["dispute_count"], 2);
    let history2: Value = srv.get(&format!("/entities/{eid}/disputes")).await.json().await.expect("history");
    assert_eq!(history2.as_array().unwrap().len(), 2);

    assert_eq!(
        srv.post("/entities/999999/dispute", &json!({"reason": "x"})).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(srv.get("/entities/999999/disputes").await.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn disputed_decision_drops_from_listings() {
    // B3 DoD under the leaf-pool contract: entity chunks are never evidence,
    // so dispute exclusion is observable on GET /decisions (status_ok).
    let srv = TestServer::start().await;
    let doc = decision_doc("Quasimoto purple", "We decided to paint the barn quasimoto purple", "Quinn");
    let source = srv.mk_folder_source("quasimoto", &[("q.md", &doc)], None).await;
    srv.sync_and_wait(source["id"].as_i64().unwrap(), "sync").await;

    let entities: Value = srv.get("/entities").await.json().await.expect("entities");
    let eid = entities[0]["id"].as_i64().unwrap();

    let before: Value = srv.get("/decisions").await.json().await.expect("decisions");
    assert!(before["decisions"].as_array().unwrap().iter().any(|d| d["entity_id"] == eid), "{before}");

    let dispute = srv.post(&format!("/entities/{eid}/dispute"), &json!({})).await;
    assert_eq!(dispute.status(), StatusCode::OK);
    assert_eq!(dispute.json::<Value>().await.expect("d")["dispute_count"], 1);

    let after: Value = srv.get("/decisions").await.json().await.expect("decisions");
    assert!(after["decisions"].as_array().unwrap().iter().all(|d| d["entity_id"] != eid), "{after}");

    // ... unless exclusion is opted out (server-level env).
    let srv_open = TestServer::start_with_env(&[("ANCHOR_QA_EXCLUDE_DISPUTED", "0")]).await;
    let doc2 = decision_doc("Quasimoto purple", "We decided to paint the barn quasimoto purple", "Quinn");
    let src2 = srv_open.mk_folder_source("quasimoto", &[("q.md", &doc2)], None).await;
    srv_open.sync_and_wait(src2["id"].as_i64().unwrap(), "sync").await;
    let entities2: Value = srv_open.get("/entities").await.json().await.expect("entities");
    let eid2 = entities2[0]["id"].as_i64().unwrap();
    srv_open.post(&format!("/entities/{eid2}/dispute"), &json!({})).await;
    let listed: Value = srv_open.get("/decisions").await.json().await.expect("decisions");
    assert!(listed["decisions"].as_array().unwrap().iter().any(|d| d["entity_id"] == eid2), "{listed}");
}

#[tokio::test]
async fn disputed_records_list_and_restore() {
    // R45.25: the Records tab lists disputed rows (`disputed=true`) and
    // restores one with PATCH status=unverified; the dispute history stays.
    let srv = TestServer::start().await;
    let d = decision_doc("Ochre lobby", "We decided to paint the lobby ochre", "Olu");
    let a = action_doc("order ochre paint samples for the lobby", "Olu");
    let source = srv.mk_folder_source("ochre", &[("d.md", &d), ("a.md", &a)], None).await;
    srv.sync_and_wait(source["id"].as_i64().unwrap(), "sync").await;

    let decisions: Value = srv.get("/decisions").await.json().await.expect("decisions");
    let did = decisions["decisions"][0]["entity_id"].as_i64().expect("decision id");
    let actions: Value = srv.get("/actions").await.json().await.expect("actions");
    let aid = actions["actions"][0]["entity_id"].as_i64().expect("action id");
    let none: Value = srv.get("/decisions?disputed=true").await.json().await.expect("none");
    assert!(none["decisions"].as_array().unwrap().is_empty(), "{none}");

    for eid in [did, aid] {
        let resp = srv.post(&format!("/entities/{eid}/dispute"), &json!({"reason": "wrong colour"})).await;
        assert_eq!(resp.status(), StatusCode::OK);
    }
    let hidden: Value = srv.get("/decisions").await.json().await.expect("hidden");
    assert!(hidden["decisions"].as_array().unwrap().iter().all(|r| r["entity_id"] != did), "{hidden}");
    let disputed: Value = srv.get("/decisions?disputed=true").await.json().await.expect("disputed");
    let rows = disputed["decisions"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "{disputed}");
    assert_eq!(rows[0]["entity_id"], did);
    assert_eq!(rows[0]["review_status"], "disputed");
    let disputed_actions: Value = srv.get("/actions?state=all&disputed=true").await.json().await.expect("disputed actions");
    assert_eq!(disputed_actions["actions"][0]["entity_id"], aid, "{disputed_actions}");

    let restored = srv.patch(&format!("/entities/{did}"), &json!({"status": "unverified"})).await;
    assert_eq!(restored.status(), StatusCode::OK);
    let back: Value = srv.get("/decisions").await.json().await.expect("back");
    assert!(back["decisions"].as_array().unwrap().iter().any(|r| r["entity_id"] == did), "{back}");
    let history: Value = srv.get(&format!("/entities/{did}/disputes")).await.json().await.expect("history");
    assert_eq!(history.as_array().unwrap().len(), 1, "restore keeps the dispute history: {history}");
}

#[tokio::test]
async fn ask_accepts_history_payload() {
    let srv = TestServer::start().await;
    let source = srv
        .mk_folder_source("fup", &[("f.md", "§1 Refund details: RFND is a return of a card payment.\n")], None)
        .await;
    srv.sync_and_wait(source["id"].as_i64().unwrap(), "sync").await;

    let resp = srv
        .post(
            "/qa",
            &json!({
                "question": "show the steps for it",
                "history": [
                    {"role": "user", "content": "refund details"},
                    {"role": "assistant", "content": "RFND is a refund event."},
                ],
            }),
        )
        .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = resp.json().await.expect("qa");
    assert!(!body["answer"].as_str().unwrap_or_default().is_empty(), "{body}");
}

#[tokio::test]
async fn qa_cites_decision_and_person_evidence() {
    let srv = TestServer::start().await;
    let source = srv
        .mk_folder_source(
            "b17qa",
            &[
                ("people.md", "Sarah owns the billing migration.\n"),
                ("decision.md", "Decision: approved moving to the new billing provider, owned by Sarah.\n"),
            ],
            None,
        )
        .await;
    srv.sync_and_wait(source["id"].as_i64().unwrap(), "sync").await;

    let resp = srv.post("/qa", &json!({"question": "Who owns the billing migration?"})).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = resp.json().await.expect("qa");
    let citations = body["citations"].as_array().unwrap();
    assert!(!citations.is_empty(), "{body}");
    let all_text: String = citations
        .iter()
        .map(|c| format!("{} {}", c["summary"].as_str().unwrap_or_default(), c["snippet"].as_str().unwrap_or_default()))
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    assert!(all_text.contains("sarah"), "person evidence missing: {all_text}");
    assert!(all_text.contains("billing"), "decision evidence missing: {all_text}");
}

#[tokio::test]
async fn hybrid_keyword_retrieval_without_embeddings() {
    let srv = TestServer::start().await;
    let source = srv
        .mk_folder_source(
            "b12",
            &[(
                "rules.md",
                "§5 ACME movement rules\nThe ACME movement must be reported with the DVSE identifier.\n\n§9 Other stuff\nUnrelated paragraph about parking regulations.\n",
            )],
            None,
        )
        .await;
    let job = srv.sync_and_wait(source["id"].as_i64().unwrap(), "sync").await;
    assert_eq!(job["status"], "done", "{job}");

    let resp = srv.post("/qa", &json!({"question": "What are the ACME movement rules?"})).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = resp.json().await.expect("qa");
    let citations = body["citations"].as_array().unwrap();
    assert!(!citations.is_empty(), "expected cited chunks: {body}");
    assert!(
        citations.iter().any(|c| {
            c["snippet"].as_str().unwrap_or_default().contains("ACME")
                || c["source_ref"].as_str().unwrap_or_default().contains("ACME")
        }),
        "{citations:?}"
    );
}
