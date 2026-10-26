//! API contract: health, system, ingest, jobs, review (black-box).
//!
//! Ports the still-valid HTTP assertions from the retired Python suite
//! (`test_smoke.py`, `test_system.py`, `test_testing_uplift.py::duplicates`).
//! Fixtures use record structure (`Decision:`/`Status:`/`Owner:`) so the
//! rules classifier yields entities deterministically (R45.8).

mod common;

use common::{TestServer, action_doc, decision_doc};
use reqwest::StatusCode;
use serde_json::{Value, json};

#[tokio::test]
async fn health_reports_components() {
    let srv = TestServer::start().await;
    let resp = srv.get("/health").await;
    assert_eq!(resp.status(), StatusCode::OK);
    let h: Value = resp.json().await.expect("health json");
    assert_eq!(h["status"], "ok");
    assert!(h["components"]["ollama"].is_string(), "{h}");
    assert!(h["components"].get("tasks").is_some(), "{h}");
    assert!(h["components"].get("failing_sources").is_some(), "{h}");
    assert!(h["components"]["pending_embeddings"].is_i64(), "{h}");
    // R45.17: the model banner branches on the embed provider.
    assert_eq!(h["components"]["embedder"]["provider"], "local", "{h}");
}

#[tokio::test]
async fn health_agent_mode_ready_without_ollama() {
    // R45.17: the model banner branches on these — agent mode needs no
    // answer model, so it stays ready with Ollama down. The harness already
    // points Ollama at a dead port.
    let srv = TestServer::start().await;
    let set = srv.put("/settings", &json!({"llm_mode": "agent"})).await;
    assert_eq!(set.status(), StatusCode::OK);
    let h: Value = srv.get("/health").await.json().await.expect("health");
    assert_eq!(h["components"]["ollama"], "offline", "{h}");
    assert_eq!(h["components"]["answer_ready"], true, "{h}");
    assert_eq!(h["components"]["embedder"]["provider"], "local", "{h}");

    // A cloud embed base flips the provider (no key needed for the label).
    let set = srv
        .put("/settings", &json!({"embed_base_url": "https://api.openai.com/v1"}))
        .await;
    assert_eq!(set.status(), StatusCode::OK);
    let h: Value = srv.get("/health").await.json().await.expect("health");
    assert_eq!(h["components"]["embedder"]["provider"], "cloud", "{h}");
}

#[tokio::test]
async fn system_status_shape() {
    let srv = TestServer::start().await;
    let status: Value = srv.get("/system/status").await.json().await.expect("status");
    assert!(!status["version"].as_str().unwrap_or_default().is_empty(), "{status}");
    assert!(status.get("data_dir").is_some(), "{status}");
    assert!(status.get("tasks").is_some(), "{status}");
    assert!(status["ollama"]["reachable"].is_boolean(), "{status}");
    assert!(status["pending_embeddings"].is_i64(), "{status}");
    assert!(status.get("answer").is_some(), "{status}");
    assert!(status.get("retrieval").is_some(), "{status}");
}

#[tokio::test]
async fn fts_and_vec_tables_exist_without_vec0() {
    let srv = TestServer::start().await;
    let db = srv.db();
    let fts: Option<String> = db
        .query_row(
            "SELECT name FROM sqlite_master WHERE type='table' AND name='chunks_fts'",
            [],
            |r| r.get(0),
        )
        .ok();
    assert_eq!(fts.as_deref(), Some("chunks_fts"));
    // Scan-backed contract: vec_chunks is a PLAIN table (never a vec0
    // virtual table — sqlite-vec is not linked) with chunk-sync triggers.
    let sql: String = db
        .query_row("SELECT sql FROM sqlite_master WHERE name = 'vec_chunks'", [], |r| r.get(0))
        .expect("vec_chunks table");
    assert!(!sql.to_uppercase().contains("VIRTUAL"), "{sql}");
    let triggers: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='trigger' AND name LIKE 'vec_chunks_%'",
            [],
            |r| r.get(0),
        )
        .expect("trigger count");
    assert_eq!(triggers, 3, "insert/delete/update sync triggers");
}

#[tokio::test]
async fn folder_ingest_flow_with_records() {
    let srv = TestServer::start().await;
    let doc = decision_doc(
        "Postpone the launch",
        "We decided to postpone the launch and Sarah owns the migration",
        "Sarah",
    );
    let source = srv.mk_folder_source("notes", &[("notes.md", &doc)], None).await;

    let job = srv.sync_and_wait(source["id"].as_i64().unwrap(), "sync").await;
    assert_eq!(job["status"], "done", "{job}");
    assert_eq!(job["result"]["items"], 1, "{job}");
    assert!(job["result"]["entities"].as_i64().unwrap_or(0) >= 1, "{job}");

    // Re-sync dedupes: nothing changed.
    let job2 = srv.sync_and_wait(source["id"].as_i64().unwrap(), "sync").await;
    assert_eq!(job2["result"]["items"], 0, "{job2}");

    let entities: Value = srv.get("/entities").await.json().await.expect("entities");
    assert!(entities.as_array().map(|a| !a.is_empty()).unwrap_or(false), "{entities}");
    let kinds: Vec<&str> = entities
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["kind"].as_str())
        .collect();
    assert!(kinds.contains(&"decision"), "{kinds:?}");

    // QA degrades gracefully without models.
    let qa = srv.post("/qa", &json!({"question": "What did we decide?"})).await;
    assert_eq!(qa.status(), StatusCode::OK);
    let qa_body: Value = qa.json().await.expect("qa json");
    assert!(!qa_body["answer"].as_str().unwrap_or_default().is_empty(), "{qa_body}");
}

#[tokio::test]
async fn delete_source_cascades_items_entities_chunks() {
    let srv = TestServer::start().await;
    let doc = decision_doc("Delete this source", "We decided to delete this source", "Sam");
    let source = srv.mk_folder_source("doomed", &[("d.md", &doc)], None).await;
    let sid = source["id"].as_i64().unwrap();
    let job = srv.sync_and_wait(sid, "sync").await;
    assert_eq!(job["status"], "done", "{job}");

    let item_id: i64 = srv
        .db()
        .query_row("SELECT id FROM ingested_items WHERE source_id = ?1", [sid], |r| r.get(0))
        .expect("item row");
    let count = |table: &str| -> i64 {
        srv.db()
            .query_row(
                &format!("SELECT COUNT(*) FROM {table} WHERE item_id = ?1"),
                [item_id],
                |r| r.get(0),
            )
            .expect("count")
    };
    assert!(count("entities") >= 1);
    assert!(count("chunks") >= 1);

    let resp = srv.delete(&format!("/sources/{sid}")).await;
    assert_eq!(resp.status(), StatusCode::OK, "DELETE /sources/{sid}");
    assert_eq!(resp.json::<Value>().await.expect("del json"), json!({"deleted": true}));

    let gone: i64 = srv
        .db()
        .query_row("SELECT COUNT(*) FROM ingested_items WHERE id = ?1", [item_id], |r| r.get(0))
        .expect("count");
    assert_eq!(gone, 0);
    assert_eq!(count("entities"), 0);
    assert_eq!(count("chunks"), 0);
}

#[tokio::test]
async fn job_history_and_progress() {
    let srv = TestServer::start().await;
    let a = decision_doc("Ship v1", "We decided to ship v1 in June", "Ann");
    let b = decision_doc("Document releases", "We decided to document the release process", "Bob");
    let source = srv.mk_folder_source("hist", &[("a.md", &a), ("b.md", &b)], None).await;
    let sid = source["id"].as_i64().unwrap();

    let job = srv.sync_and_wait(sid, "sync").await;
    assert_eq!(job["total"], 2, "{job}");
    assert_eq!(job["processed"], job["total"], "{job}");

    let history: Value = srv
        .get(&format!("/sources/jobs?source_id={sid}"))
        .await
        .json()
        .await
        .expect("history");
    let ids: Vec<i64> = history
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|j| j["id"].as_i64())
        .collect();
    assert!(ids.contains(&job["id"].as_i64().unwrap()), "{history}");
    assert_eq!(history[0]["kind"], "sync", "{history}");
    assert_eq!(history[0]["status"], "done", "{history}");
    assert!(history[0]["result"]["entities"].as_i64().unwrap_or(0) >= 1, "{history}");
}

#[tokio::test]
async fn reclassify_runs_as_job_and_rebuilds_entities() {
    let srv = TestServer::start().await;
    let doc = decision_doc("Adopt the new stack", "We decided to adopt the new stack", "Ada");
    let source = srv.mk_folder_source("recl", &[("r.md", &doc)], None).await;
    let sid = source["id"].as_i64().unwrap();
    srv.sync_and_wait(sid, "sync").await;
    let before: Value = srv.get("/entities").await.json().await.expect("entities");
    assert!(!before.as_array().unwrap().is_empty());

    let job = srv.sync_and_wait(sid, "reclassify").await;
    assert_eq!(job["status"], "done", "{job}");
    assert_eq!(job["kind"], "reclassify", "{job}");
    assert!(job["result"]["entities"].as_i64().unwrap_or(0) >= 1, "{job}");

    let after: Value = srv.get("/entities").await.json().await.expect("entities");
    assert!(!after.as_array().unwrap().is_empty());
}

#[tokio::test]
async fn review_endpoints_verify_entity() {
    let srv = TestServer::start().await;
    let doc = decision_doc("Ship v1", "We decided to ship v1 in June", "Ann");
    let source = srv.mk_folder_source("notes", &[("a.md", &doc)], None).await;
    srv.sync_and_wait(source["id"].as_i64().unwrap(), "sync").await;

    let low = srv.get("/review/low-confidence").await;
    assert_eq!(low.status(), StatusCode::OK);

    let entities: Value = srv.get("/entities").await.json().await.expect("entities");
    let eid = entities[0]["id"].as_i64().unwrap();
    let patch = srv.patch(&format!("/entities/{eid}"), &json!({"status": "verified"})).await;
    assert_eq!(patch.status(), StatusCode::OK);
    assert_eq!(patch.json::<Value>().await.expect("patch")["status"], "verified");
}

#[tokio::test]
async fn sync_error_tracking_and_structured_event() {
    let srv = TestServer::start().await;
    let bad_path = srv.work.path().join("nope");
    let resp = srv
        .post(
            "/sources",
            &json!({
                "connector": "folder",
                "name": "bad",
                "config": {"path": bad_path.to_string_lossy()},
            }),
        )
        .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let source: Value = resp.json().await.expect("source");
    let sid = source["id"].as_i64().unwrap();

    let job = srv.sync_and_wait(sid, "sync").await;
    assert_eq!(job["status"], "failed", "{job}");
    assert!(job["error"].as_str().unwrap_or_default().contains("does not exist"), "{job}");

    let sources: Value = srv.get("/sources").await.json().await.expect("sources");
    let failing = sources
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == sid)
        .expect("own source");
    assert_eq!(failing["error_count"], 1, "{failing}");
    assert!(failing["last_error"].as_str().unwrap_or_default().contains("does not exist"), "{failing}");

    let events: Value = srv
        .get("/system/errors?component=pipeline")
        .await
        .json()
        .await
        .expect("events");
    let mine: Vec<&Value> = events
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["source_id"] == sid)
        .collect();
    assert!(!mine.is_empty(), "no pipeline event for the failed sync: {events}");
    assert!(mine[0]["message"].as_str().unwrap_or_default().contains("sync failed for source"), "{mine:?}");
    assert!(mine[0]["detail"].as_str().unwrap_or_default().contains("does not exist"), "{mine:?}");
}

#[tokio::test]
async fn job_404_and_cancel_404() {
    let srv = TestServer::start().await;
    assert_eq!(srv.get("/sources/jobs/999999").await.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        srv.post_empty("/sources/jobs/999999/cancel").await.status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn cancel_running_job_then_409() {
    let srv = TestServer::start().await;
    // A big doc keeps reclassification in flight long enough to cancel.
    let big = format!("We decided to ship the platform in June. {}", "x".repeat(40))
        .repeat(400);
    let source = srv.mk_folder_source("cancel", &[("big.md", &big)], None).await;
    let sid = source["id"].as_i64().unwrap();
    srv.sync_and_wait(sid, "sync").await;

    let resp = srv.post_empty(&format!("/sources/{sid}/reclassify")).await;
    assert_eq!(resp.status(), StatusCode::ACCEPTED);
    let started: Value = resp.json().await.expect("job");
    let job_id = started["id"].as_i64().unwrap();

    let cancel = srv.post_empty(&format!("/sources/jobs/{job_id}/cancel")).await;
    assert_eq!(cancel.status(), StatusCode::OK, "cancel running job");
    assert_eq!(cancel.json::<Value>().await.expect("cancel")["cancelled"], true);

    let terminal = srv.wait_job(job_id).await;
    assert_eq!(terminal["status"], "cancelled", "{terminal}");

    assert_eq!(
        srv.post_empty(&format!("/sources/jobs/{job_id}/cancel")).await.status(),
        StatusCode::CONFLICT
    );
}

#[tokio::test]
async fn orphaned_running_job_cancelled_via_endpoint_and_restart() {
    let mut srv = TestServer::start().await;
    let doc = decision_doc("Seed", "We decided to seed the orphan test", "Otto");
    let source = srv.mk_folder_source("orphan", &[("s.md", &doc)], None).await;
    let sid = source["id"].as_i64().unwrap();

    // A running row with no live task behind it must cancel, not 409 forever.
    srv.db()
        .execute(
            "INSERT INTO jobs (source_id, kind, status) VALUES (?1, 'reclassify', 'running')",
            [sid],
        )
        .expect("plant orphan");
    let orphan: i64 = srv
        .db()
        .query_row("SELECT id FROM jobs WHERE source_id = ?1 ORDER BY id DESC LIMIT 1", [sid], |r| {
            r.get(0)
        })
        .expect("orphan id");
    let cancel = srv.post_empty(&format!("/sources/jobs/{orphan}/cancel")).await;
    assert_eq!(cancel.status(), StatusCode::OK, "cancel orphaned job");
    let status: String = srv
        .db()
        .query_row("SELECT status FROM jobs WHERE id = ?1", [orphan], |r| r.get(0))
        .expect("status");
    assert_eq!(status, "cancelled");

    // A restart sweeps running rows left by the dead process.
    srv.db()
        .execute(
            "INSERT INTO jobs (source_id, kind, status) VALUES (?1, 'sync', 'running')",
            [sid],
        )
        .expect("plant crash orphan");
    let crash: i64 = srv
        .db()
        .query_row("SELECT id FROM jobs WHERE source_id = ?1 ORDER BY id DESC LIMIT 1", [sid], |r| {
            r.get(0)
        })
        .expect("crash id");
    srv.restart(&[]).await;
    let fetched: Value = srv
        .get(&format!("/sources/jobs/{crash}"))
        .await
        .json()
        .await
        .expect("job");
    assert_eq!(fetched["status"], "cancelled", "{fetched}");
}

#[tokio::test]
async fn second_sync_while_running_returns_409_with_live_job_id() {
    // R30.1: at most one active sync/reclassify per source. A planted
    // `running` row stands in for a live job (deterministic — no timing).
    let srv = TestServer::start().await;
    let doc = decision_doc("Seed", "We decided to seed the dedup test", "Otto");
    let source = srv.mk_folder_source("dedup", &[("s.md", &doc)], None).await;
    let sid = source["id"].as_i64().unwrap();

    srv.db()
        .execute(
            "INSERT INTO jobs (source_id, kind, status) VALUES (?1, 'sync', 'running')",
            [sid],
        )
        .expect("plant running");
    let live: i64 = srv
        .db()
        .query_row("SELECT id FROM jobs WHERE source_id = ?1 ORDER BY id DESC LIMIT 1", [sid], |r| {
            r.get(0)
        })
        .expect("live id");

    // sync+sync → 409 naming the live job, no new row
    let resp = srv.post_empty(&format!("/sources/{sid}/sync")).await;
    assert_eq!(resp.status(), StatusCode::CONFLICT);
    let body: Value = resp.json().await.expect("409 json");
    assert_eq!(body["job_id"], live, "{body}");
    assert!(
        body["detail"].as_str().unwrap_or_default().contains(&format!("#{live}")),
        "{body}"
    );
    let n: i64 = srv
        .db()
        .query_row("SELECT COUNT(*) FROM jobs WHERE source_id = ?1", [sid], |r| r.get(0))
        .expect("count");
    assert_eq!(n, 1, "a 409 must not add a row");

    // sync+reclassify (cross-kind) → same 409, same live job
    let resp = srv.post_empty(&format!("/sources/{sid}/reclassify")).await;
    assert_eq!(resp.status(), StatusCode::CONFLICT);
    let body: Value = resp.json().await.expect("409 json");
    assert_eq!(body["job_id"], live, "{body}");

    // cancelling the live job frees the slot: the next sync starts clean
    let cancel = srv.post_empty(&format!("/sources/jobs/{live}/cancel")).await;
    assert_eq!(cancel.status(), StatusCode::OK);
    let job = srv.sync_and_wait(sid, "sync").await;
    assert_eq!(job["status"], "done", "{job}");
}

#[tokio::test]
async fn duplicate_sync_while_pending_returns_409() {
    // R30.1: pending counts as active. Both global slots are parked with
    // orphan running rows for other sources, so the new sync stays pending
    // instead of racing to done.
    let srv = TestServer::start().await;
    let doc = decision_doc("Seed", "We decided to seed the pending test", "Otto");
    let a = srv.mk_folder_source("pa", &[("a.md", &doc)], None).await;
    let b = srv.mk_folder_source("pb", &[("b.md", &doc)], None).await;
    let c = srv.mk_folder_source("pc", &[("c.md", &doc)], None).await;
    let (sa, sb, sc) = (
        a["id"].as_i64().unwrap(),
        b["id"].as_i64().unwrap(),
        c["id"].as_i64().unwrap(),
    );
    for sid in [sa, sb] {
        srv.db()
            .execute(
                "INSERT INTO jobs (source_id, kind, status) VALUES (?1, 'sync', 'running')",
                [sid],
            )
            .expect("park a slot");
    }
    let parked: Vec<i64> = srv
        .db()
        .prepare("SELECT id FROM jobs WHERE status = 'running'")
        .expect("prepare")
        .query_map([], |r| r.get(0))
        .expect("query")
        .filter_map(|r| r.ok())
        .collect();
    assert_eq!(parked.len(), 2);

    // slots full → the new sync is accepted but stays pending
    let resp = srv.post_empty(&format!("/sources/{sc}/sync")).await;
    assert_eq!(resp.status(), StatusCode::ACCEPTED);
    let first: Value = resp.json().await.expect("job");
    assert_eq!(first["status"], "pending", "{first}");
    let jid = first["id"].as_i64().unwrap();

    // a duplicate while pending → 409 naming it, no new row
    let resp = srv.post_empty(&format!("/sources/{sc}/sync")).await;
    assert_eq!(resp.status(), StatusCode::CONFLICT);
    let body: Value = resp.json().await.expect("409 json");
    assert_eq!(body["job_id"], jid, "{body}");

    // freeing the slots promotes the pending job and it finishes
    for pid in parked {
        let cancel = srv.post_empty(&format!("/sources/jobs/{pid}/cancel")).await;
        assert_eq!(cancel.status(), StatusCode::OK);
    }
    let done = srv.wait_job(jid).await;
    assert_eq!(done["status"], "done", "{done}");
}

#[tokio::test]
async fn merge_cleans_references() {
    let srv = TestServer::start().await;
    let d0 = decision_doc("Adopt stack zero", "We decided to adopt stack number zero", "Zed");
    let d1 = decision_doc("Adopt stack one", "We decided to adopt stack number one", "Uma");
    let source = srv.mk_folder_source("merge", &[("d0.md", &d0), ("d1.md", &d1)], None).await;
    srv.sync_and_wait(source["id"].as_i64().unwrap(), "sync").await;

    let entities: Value = srv.get("/entities").await.json().await.expect("entities");
    assert!(entities.as_array().unwrap().len() >= 2, "{entities}");
    let (a, b) = (
        entities[0]["id"].as_i64().unwrap(),
        entities[1]["id"].as_i64().unwrap(),
    );
    srv.db()
        .execute(
            "INSERT INTO merge_actions (entity_a_id, entity_b_id, status) VALUES (?1, ?2, 'proposed')",
            [a, b],
        )
        .expect("plant proposal");
    let proposal: i64 = srv
        .db()
        .query_row("SELECT id FROM merge_actions ORDER BY id DESC LIMIT 1", [], |r| r.get(0))
        .expect("proposal id");
    srv.db()
        .execute(
            "INSERT INTO relationships (from_entity_id, to_entity_id, kind) VALUES (?1, ?2, 'related')",
            [a, b],
        )
        .expect("plant relationship");

    let resp = srv.post("/review/merge", &json!({"proposal_id": proposal, "decision": "merge"})).await;
    assert_eq!(resp.status(), StatusCode::OK, "POST /review/merge");
    assert_eq!(resp.json::<Value>().await.expect("merge")["merged"], true);
}

#[tokio::test]
async fn full_document_chunking_with_citations() {
    let srv = TestServer::start().await;
    let mut paras = vec![decision_doc(
        "Launch the beta",
        "We decided to launch the beta in June",
        "Beth",
    )];
    for i in 1..29 {
        paras.push(format!("Paragraph number {i} discussing the feature set in detail."));
    }
    paras.push("Action item: finish the remaining rollout before the launch.".to_string());
    let source = srv.mk_folder_source("big", &[("big.md", &paras.join("\n\n"))], None).await;
    let sid = source["id"].as_i64().unwrap();
    srv.sync_and_wait(sid, "sync").await;

    let db = srv.db();
    let (item_id, text): (i64, String) = db
        .query_row(
            "SELECT id, text FROM ingested_items WHERE source_id = ?1",
            [sid],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("item");
    assert!(!text.is_empty(), "raw text must be stored");
    let doc_chunks: Vec<String> = db
        .prepare("SELECT content FROM chunks WHERE item_id = ?1 AND entity_id IS NULL")
        .expect("stmt")
        .query_map([item_id], |r| r.get(0))
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("rows");
    assert!(doc_chunks.len() >= 2, "long doc must chunk: {}", doc_chunks.len());
    assert!(doc_chunks.iter().any(|c| c.contains("Paragraph number 28")), "tail must be chunked");
    let entities: i64 = db
        .query_row("SELECT COUNT(*) FROM entities WHERE item_id = ?1", [item_id], |r| r.get(0))
        .expect("count");
    assert!(entities >= 1, "record still yields entities");

    let qa = srv.post("/qa", &json!({"question": "feature set"})).await;
    assert_eq!(qa.status(), StatusCode::OK);
    let body: Value = qa.json().await.expect("qa");
    assert!(!body["citations"].as_array().unwrap().is_empty(), "{body}");
}

#[tokio::test]
async fn qa_without_hits_records_degraded_warnings() {
    // R10.5: the qa warnings fire only when retrieval actually degrades
    // (empty hits) — not on every call.
    let srv = TestServer::start().await;
    let qa = srv.post("/qa", &json!({"question": "zxqy wibble nonexistent term"})).await;
    assert_eq!(qa.status(), StatusCode::OK);
    let body: Value = qa.json().await.expect("qa");
    assert!(body["citations"].as_array().unwrap().is_empty(), "{body}");
    assert!(body["answer"].as_str().unwrap_or_default().contains("No relevant knowledge"), "{body}");

    let events: Value = srv
        .get("/system/errors?component=qa")
        .await
        .json()
        .await
        .expect("events");
    let messages: Vec<&str> = events
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["message"].as_str())
        .collect();
    assert!(messages.iter().any(|m| m.contains("retrieval degraded")), "{messages:?}");
    assert!(messages.iter().any(|m| m.contains("answer generation")), "{messages:?}");
}

#[tokio::test]
async fn get_config_redacts_secrets() {
    let srv = TestServer::start().await;
    let resp = srv
        .post(
            "/sources",
            &json!({
                "connector": "jira",
                "name": "j",
                "config": {
                    "base_url": "https://x.atlassian.net",
                    "email": "a@b.c",
                    "token": "supersecret",
                    "project": "PM",
                },
            }),
        )
        .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let source: Value = resp.json().await.expect("source");
    let config: Value = srv
        .get(&format!("/sources/{}/config", source["id"].as_i64().unwrap()))
        .await
        .json()
        .await
        .expect("config");
    assert_eq!(config["token"], "***set***", "{config}");
    assert!(!config.to_string().contains("supersecret"), "{config}");
}

#[tokio::test]
async fn logs_list_and_download() {
    let srv = TestServer::start().await;
    let files: Value = srv.get("/system/logs").await.json().await.expect("logs");
    assert!(files.as_array().unwrap().iter().any(|f| f["name"] == "anchorcore.log"), "{files}");

    let resp = srv.get("/system/logs/anchorcore.log").await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(resp.headers()["content-type"].to_str().unwrap_or_default().contains("text/plain"));

    assert_eq!(srv.get("/system/logs/anchorcore.log.5").await.status(), StatusCode::NOT_FOUND);
    assert_eq!(srv.get("/system/logs/..%2F..%2Fetc%2Fpasswd").await.status(), StatusCode::NOT_FOUND);
    assert_eq!(srv.get("/system/logs/evil.txt").await.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn onboarding_state_delta_on_fresh_server() {
    let srv = TestServer::start().await;
    let before: Value = srv.get("/system/onboarding").await.json().await.expect("onboarding");
    // Fresh server = genuinely empty DB: the wizard must trigger.
    assert_eq!(before["needs_wizard"], true, "{before}");
    assert_eq!(before["sources_count"], 0, "{before}");
    assert!(before["ollama"]["missing_models"].is_array(), "{before}");
    assert!(before["answer_ready"].is_boolean(), "{before}");
    assert!(before["sample"]["available"].is_boolean(), "{before}");
    assert!(["ollama", "configured", "missing"].contains(&before["answer_provider"].as_str().unwrap_or("")), "{before}");

    let doc = decision_doc("Keep going", "We decided to keep going", "Wiz");
    srv.mk_folder_source("wiz", &[("d.md", &doc)], None).await;

    let after: Value = srv.get("/system/onboarding").await.json().await.expect("onboarding");
    assert_eq!(after["sources_count"], 1, "{after}");
    assert_eq!(after["needs_wizard"], false, "{after}");
}

/// R45.6: the wizard's local-model picker reads RAM + a sized catalog from
/// onboarding; exactly one model is recommended and it names the field.
#[tokio::test]
async fn onboarding_reports_ram_and_a_sized_answer_model() {
    let srv = TestServer::start().await;
    let v: Value = srv.get("/system/onboarding").await.json().await.expect("onboarding");
    let machine = &v["machine"];
    let ram = machine["total_memory_bytes"].as_u64();
    if cfg!(any(target_os = "macos", target_os = "linux", windows)) {
        assert!(ram.unwrap_or(0) > 512 * 1024 * 1024, "RAM should be known here: {machine}");
    }
    let models = machine["answer_models"].as_array().expect("answer_models");
    assert!(models.len() >= 3, "{machine}");
    let recommended: Vec<&Value> = models.iter().filter(|m| m["recommended"] == true).collect();
    assert_eq!(recommended.len(), 1, "{machine}");
    assert_eq!(recommended[0]["model"], machine["recommended_answer_model"], "{machine}");
    for m in models {
        assert!(m["model"].as_str().is_some_and(|s| s.contains(':')), "explicit size tag: {m}");
        assert!(m["download_gb"].as_f64().is_some() && m["min_ram_gb"].as_f64().is_some(), "{m}");
        if let Some(bytes) = ram {
            // The recommendation never needs more RAM than the machine has.
            if m["recommended"] == true && m["min_ram_gb"].as_f64().unwrap() > 0.0 {
                assert!(m["min_ram_gb"].as_f64().unwrap() <= bytes as f64 / 1073741824.0 + 0.5, "{m}");
            }
        }
    }
}

#[tokio::test]
async fn duplicates_paginate_and_filter() {
    let srv = TestServer::start().await;
    let source = srv.mk_folder_source("dups", &[("seed.md", "seed")], None).await;
    let sid = source["id"].as_i64().unwrap();

    // Near-identical entities, planted directly (the rules classifier is not
    // guaranteed to produce duplicates from a corpus).
    let summaries = [
        "Annual budget approved for the fiscal year by the board",
        "Fiscal year annual budget approved by the board",
        "Hiring freeze stays in place until the third quarter",
        "The hiring freeze remains until Q3 of this year",
        "New Berlin office opens in January for the sales team",
        "The Berlin office opens in January for sales",
    ];
    {
        let db = srv.db();
        for (i, s) in summaries.iter().enumerate() {
            db.execute(
                "INSERT INTO ingested_items (source_id, external_id, title, text, content_hash) VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![sid, format!("dup-{i}"), s, s, format!("hash-{i}")],
            )
            .expect("plant item");
            let item_id = db.last_insert_rowid();
            db.execute(
                "INSERT INTO entities (item_id, kind, summary, reasoning, confidence, author, source_ref, status) VALUES (?1, 'decision', ?2, 'test fixture', 0.8, 'fixture', 'dups-fixture', 'unverified')",
                rusqlite::params![item_id, s],
            )
            .expect("plant entity");
        }
    }

    let all: Value = srv.get("/review/duplicates").await.json().await.expect("dups");
    assert!(all.as_array().map(|a| !a.is_empty()).unwrap_or(false), "{all}");

    let limited: Value = srv
        .get("/review/duplicates?limit=1")
        .await
        .json()
        .await
        .expect("limited");
    assert!(limited.as_array().unwrap().len() <= 1, "{limited}");

    let page0: Value = srv
        .get("/review/duplicates?limit=1&offset=0")
        .await
        .json()
        .await
        .expect("page0");
    let page1: Value = srv
        .get("/review/duplicates?limit=1&offset=1")
        .await
        .json()
        .await
        .expect("page1");
    if !page0.as_array().unwrap().is_empty() && !page1.as_array().unwrap().is_empty() {
        assert_ne!(page0[0]["id"], page1[0]["id"], "offset must skip the previous page");
    }

    let by_kind: Value = srv
        .get("/review/duplicates?kind=decision")
        .await
        .json()
        .await
        .expect("by_kind");
    assert!(by_kind.is_array(), "{by_kind}");
    assert!(
        by_kind.as_array().unwrap().iter().all(|p| p.get("entity_a_id").is_some() && p.get("entity_b_id").is_some()),
        "{by_kind}"
    );

    let huge: Value = srv
        .get("/review/duplicates?limit=9999")
        .await
        .json()
        .await
        .expect("huge");
    assert!(huge.as_array().unwrap().len() <= 100, "limit must clamp");
}

#[test]
fn action_doc_helper_shape() {
    // Pin: the shared action fixture carries the bucket marker + owner line
    // the record parser needs (see common::action_doc).
    let doc = action_doc("migrate the billing database", "Priya");
    assert!(doc.contains("Attendees:"));
    assert!(doc.contains("## Action items"));
    assert!(doc.contains("Priya: migrate the billing database"));
    assert!(doc.contains("Owner: Priya."));
}
