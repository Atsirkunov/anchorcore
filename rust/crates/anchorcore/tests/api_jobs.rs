//! R30.4: killing jobs — prompt mid-sync cancel with partial state kept,
//! instant pending cancel, 409 on terminal jobs, promote-after-cancel.

mod common;

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::TestServer;
use reqwest::StatusCode;
use serde_json::{json, Value};

/// Fake OpenAI-compatible embedder: answers the first `answer_first`
/// requests, then hangs (stands in for a stuck provider) until `release`
/// is set — after that it answers everything.
struct FakeEmbedder {
    base: String,
    calls: Arc<AtomicUsize>,
    release: Arc<AtomicBool>,
}

async fn fake_embedder(answer_first: usize) -> FakeEmbedder {
    let calls = Arc::new(AtomicUsize::new(0));
    let release = Arc::new(AtomicBool::new(false));
    let (c, r) = (calls.clone(), release.clone());
    let handler = move |axum::Json(body): axum::Json<Value>| {
        let (c, r) = (c.clone(), r.clone());
        async move {
            let n = c.fetch_add(1, Ordering::SeqCst) + 1;
            while n > answer_first && !r.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            let count = body["input"].as_array().map(Vec::len).unwrap_or(1);
            let data: Vec<Value> = (0..count)
                .map(|i| json!({"index": i, "embedding": [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, (i as f64) / 100.0]}))
                .collect();
            axum::Json(json!({ "data": data }))
        }
    };
    let app = axum::Router::new().route("/v1/embeddings", axum::routing::post(handler));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    FakeEmbedder { base, calls, release }
}

fn corpus(n: usize) -> Vec<(String, String)> {
    (0..n)
        .map(|i| {
            (
                format!("note{i:02}.md"),
                format!("# Note {i}\n\nThe harbour crane number {i} lifts container batch {i} before the tide turns at dock {i}.\n"),
            )
        })
        .collect()
}

async fn wait_for(what: &str, deadline: Duration, mut ok: impl AsyncFnMut() -> bool) {
    let end = Instant::now() + deadline;
    while !ok().await {
        assert!(Instant::now() < end, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

fn log_has(srv: &TestServer, needle: &str) -> bool {
    std::fs::read_to_string(srv.dir.path().join("anchorcore.log")).is_ok_and(|s| s.contains(needle))
}

#[tokio::test]
async fn mid_sync_cancel_stops_within_one_call_and_keeps_partial_state() {
    let fake = fake_embedder(3).await;
    let srv = TestServer::start_with_env(&[
        ("ANCHOR_EMBED_BASE_URL", &fake.base),
        ("ANCHOR_EMBED_PROVIDER", "ollama"),
        // one-chunk docs score IDF 0 against themselves; embed them anyway
        ("ANCHOR_EMBED_MIN_SIGNAL", "0"),
    ])
    .await;
    let files = corpus(12);
    let refs: Vec<(&str, &str)> = files.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    let sid = srv.mk_folder_source("kill", &refs, None).await["id"].as_i64().unwrap();

    let resp = srv.post_empty(&format!("/sources/{sid}/sync")).await;
    assert_eq!(resp.status(), StatusCode::ACCEPTED);
    let jid = resp.json::<Value>().await.unwrap()["id"].as_i64().unwrap();

    // three docs embed, the fourth embed call hangs in flight
    wait_for("4th embed call in flight", Duration::from_secs(60), async || {
        fake.calls.load(Ordering::SeqCst) >= 4
    })
    .await;
    let before: Value = srv.get_json(&format!("/sources/jobs/{jid}")).await;
    assert_eq!(before["status"], "running", "{before}");
    assert_eq!(before["processed"], 3, "{before}");

    let stop = Instant::now();
    let cancel = srv.post_empty(&format!("/sources/jobs/{jid}/cancel")).await;
    assert_eq!(cancel.status(), StatusCode::OK);
    let body: Value = cancel.json().await.unwrap();
    assert_eq!((body["cancelled"].as_bool(), body["was"].as_str()), (Some(true), Some("running")), "{body}");
    assert_eq!((body["processed"].as_i64(), body["total"].as_i64()), (Some(3), Some(12)), "{body}");

    // the task itself stops (not just the row): it drops the hung call
    let needle = format!("job {jid} (sync) stopped: cancelled, 3/12 docs kept");
    wait_for("the job task to stop", Duration::from_secs(5), async || log_has(&srv, &needle)).await;
    let latency = stop.elapsed();
    eprintln!("R30.4 Stop -> stopped: {} ms (hung provider call in flight)", latency.as_millis());
    assert!(latency < Duration::from_secs(3), "cancel took {latency:?}");

    let job: Value = srv.get_json(&format!("/sources/jobs/{jid}")).await;
    assert_eq!(job["status"], "cancelled", "{job}");
    assert_eq!((job["processed"].as_i64(), job["total"].as_i64()), (Some(3), Some(12)), "processed count preserved: {job}");
    assert!(job["finished_at"].is_string(), "{job}");

    // partial ingestion is kept: the three finished docs stay processed
    let done: i64 = srv
        .db()
        .query_row("SELECT COUNT(*) FROM ingested_items WHERE source_id=?1 AND processed_hash IS NOT NULL", [sid], |r| r.get(0))
        .unwrap();
    assert_eq!(done, 3);

    // a second cancel is a conflict
    assert_eq!(srv.post_empty(&format!("/sources/jobs/{jid}/cancel")).await.status(), StatusCode::CONFLICT);

    // the next sync starts clean and finishes the rest
    fake.release.store(true, Ordering::SeqCst);
    let next = srv.sync_and_wait(sid, "sync").await;
    assert_eq!(next["status"], "done", "{next}");
    let done: i64 = srv
        .db()
        .query_row("SELECT COUNT(*) FROM ingested_items WHERE source_id=?1 AND processed_hash IS NOT NULL", [sid], |r| r.get(0))
        .unwrap();
    assert_eq!(done, 12);
}

#[tokio::test]
async fn pending_cancel_is_instant_and_freed_slot_promotes_next() {
    let srv = TestServer::start().await;
    let doc = common::decision_doc("Seed", "We decided to seed the kill test", "Otto");
    let mut sids = Vec::new();
    for name in ["ka", "kb", "kc", "kd"] {
        sids.push(srv.mk_folder_source(name, &[("a.md", &doc)], None).await["id"].as_i64().unwrap());
    }
    // park both global slots with orphan running rows
    for sid in &sids[..2] {
        srv.db()
            .execute("INSERT INTO jobs (source_id, kind, status, started_at) VALUES (?1, 'sync', 'running', datetime('now'))", [sid])
            .unwrap();
    }
    let parked: i64 = srv.db().query_row("SELECT MIN(id) FROM jobs WHERE status='running'", [], |r| r.get(0)).unwrap();

    let mut queued = Vec::new();
    for sid in &sids[2..] {
        let resp = srv.post_empty(&format!("/sources/{sid}/sync")).await;
        assert_eq!(resp.status(), StatusCode::ACCEPTED);
        let job: Value = resp.json().await.unwrap();
        assert_eq!(job["status"], "pending", "{job}");
        queued.push(job["id"].as_i64().unwrap());
    }

    // pending cancel: instant, never started
    let t = Instant::now();
    let cancel = srv.post_empty(&format!("/sources/jobs/{}/cancel", queued[1])).await;
    assert_eq!(cancel.status(), StatusCode::OK);
    assert_eq!(cancel.json::<Value>().await.unwrap()["was"], "pending");
    let job: Value = srv.get_json(&format!("/sources/jobs/{}", queued[1])).await;
    assert_eq!(job["status"], "cancelled", "{job}");
    assert!(job["started_at"].is_null(), "a pending job never starts: {job}");
    assert!(t.elapsed() < Duration::from_secs(2), "pending cancel took {:?}", t.elapsed());
    assert_eq!(srv.post_empty(&format!("/sources/jobs/{}/cancel", queued[1])).await.status(), StatusCode::CONFLICT);

    // cancelling a running job frees its slot: the first queued job promotes and finishes
    assert_eq!(srv.post_empty(&format!("/sources/jobs/{parked}/cancel")).await.status(), StatusCode::OK);
    let promoted = srv.wait_job(queued[0]).await;
    assert_eq!(promoted["status"], "done", "{promoted}");

    // 409 on a done job, and the cancelled one stayed cancelled
    assert_eq!(srv.post_empty(&format!("/sources/jobs/{}/cancel", queued[0])).await.status(), StatusCode::CONFLICT);
    let still: Value = srv.get_json(&format!("/sources/jobs/{}", queued[1])).await;
    assert_eq!(still["status"], "cancelled", "{still}");
}
