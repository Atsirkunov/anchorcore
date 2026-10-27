//! API contract: the R35.5 Sensitive gate — one caller rule on every surface
//! that hands Sensitive bytes to something other than the person at the UI.
//!
//! The SPA sends `X-Anchor-Client: ui` and is never gated; every other caller
//! (MCP, skill, curl) is gated unless the provider is trusted (the R35.6
//! switches replace this opt-in later). `public_only` / `/qa/public` drop
//! effective-Sensitive rows whatever the provider or caller. Single-item byte
//! reads refuse with 403 `refusal: "gated_provider"` (+ one audit event);
//! multi-item lists drop the gated rows (+ one audit event).

mod common;

use std::io::Write;
use std::process::{Command, Stdio};

use common::{action_doc, decision_doc, TestServer};
use reqwest::StatusCode;
use serde_json::{Value, json};

const RUNBOOK: &str = "# Runbook\n\nIntro line about the billing cutover.\n\n## Rollback\n\nStep one: drain the settlement queue.\nStep two: revert the routing flag.\n";
const SENSITIVE_WORD: &str = "moonvault";

async fn status_and_body(srv: &TestServer, path: &str) -> (StatusCode, Value) {
    let resp = srv.get(path).await;
    let status = resp.status();
    (status, resp.json().await.unwrap_or(Value::Null))
}

async fn status_and_body_ui(srv: &TestServer, path: &str) -> (StatusCode, Value) {
    let resp = srv.get_ui(path).await;
    let status = resp.status();
    (status, resp.json().await.unwrap_or(Value::Null))
}

fn gate_events(srv: &TestServer) -> i64 {
    srv.db()
        .query_row("SELECT COUNT(*) FROM system_events WHERE message LIKE 'answer gate:%'", [], |r| r.get(0))
        .unwrap()
}

/// Sensitive + internal folder sources, each with the runbook + one decision
/// doc + one action doc. Returns (sensitive item id, internal item id).
async fn synced_pair(srv: &TestServer, name: &str) -> (i64, i64) {
    let sens_doc = decision_doc(
        &format!("{SENSITIVE_WORD} rotation"),
        &format!("We rotate the {SENSITIVE_WORD} seals quarterly"),
        "Hana",
    );
    let sens_act = action_doc(&format!("rotate the {SENSITIVE_WORD} seals"), "Hana");
    let sens = srv
        .mk_folder_source(
            &format!("{name}-sens"),
            &[("runbook.md", RUNBOOK), ("seal.md", &sens_doc), ("tasks.md", &sens_act)],
            Some("sensitive"),
        )
        .await;
    let open_doc = decision_doc("Standup time", "We hold the standup at nine thirty", "Ivy");
    let open_act = action_doc("book the offsite room", "Ivy");
    let open = srv
        .mk_folder_source(
            &format!("{name}-open"),
            &[("runbook.md", RUNBOOK), ("standup.md", &open_doc), ("tasks.md", &open_act)],
            None,
        )
        .await;
    for sid in [sens["id"].as_i64().unwrap(), open["id"].as_i64().unwrap()] {
        let job = srv.sync_and_wait(sid, "sync").await;
        assert_eq!(job["status"], "done", "{job}");
    }
    let item_of = |sid: i64, path: &str| {
        srv.db()
            .query_row(
                "SELECT id FROM ingested_items WHERE source_id = ?1 AND external_id = ?2",
                rusqlite::params![sid, path],
                |r| r.get(0),
            )
            .expect("item id")
    };
    (item_of(sens["id"].as_i64().unwrap(), "runbook.md"), item_of(open["id"].as_i64().unwrap(), "runbook.md"))
}

#[tokio::test]
async fn whole_file_read_gated_for_agents_open_for_ui_and_trust() {
    // Default harness provider is an untrusted cloud URL.
    let srv = TestServer::start().await;
    let (sens_item, open_item) = synced_pair(&srv, "gate-whole").await;
    // A marked document on an ordinary source gates like a sensitive label.
    let marked = srv.patch(&format!("/items/{open_item}"), &json!({"sensitive_override": true})).await;
    assert_eq!(marked.status(), StatusCode::OK);

    for item in [sens_item, open_item] {
        let (status, body) = status_and_body(&srv, &format!("/items/{item}")).await;
        assert_eq!((status, body["refusal"].as_str()), (StatusCode::FORBIDDEN, Some("gated_provider")), "item {item}: {body}");
        let (status, body) = status_and_body_ui(&srv, &format!("/items/{item}")).await;
        assert_eq!(status, StatusCode::OK, "UI reads everything: {body}");
        assert!(body["content"].as_str().unwrap_or_default().contains("drain the settlement queue"), "{body}");
        // The window form stays gated the same way (R36.15), UI exempt.
        let (status, body) = status_and_body(&srv, &format!("/items/{item}?lines=1-2")).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "window {item}: {body}");
        let v = srv.get_ui_json(&format!("/items/{item}?lines=1-2")).await;
        assert!(v["content"].as_str().unwrap_or_default().contains("Runbook"), "{v}");
    }
    assert!(gate_events(&srv) >= 4, "each refusal is audited");

    // The trust opt-in lifts the agent gate (R35.6 switches replace this).
    let trusted = TestServer::start_with_env(&[("ANCHOR_CLOUD_TRUST", "1")]).await;
    let (sens_item, _) = synced_pair(&trusted, "gate-whole-trusted").await;
    let (status, body) = status_and_body(&trusted, &format!("/items/{sens_item}")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["content"].as_str().unwrap_or_default().contains("drain the settlement queue"), "{body}");
}

#[tokio::test]
async fn node_text_and_section_chunks_follow_the_caller_rule() {
    let srv = TestServer::start().await;
    let (sens_item, _) = synced_pair(&srv, "gate-nodes").await;
    let outline = srv.get_ui_json(&format!("/items/{sens_item}/outline")).await;
    let rollback = outline["sections"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["title"] == "Rollback")
        .unwrap_or_else(|| panic!("no Rollback section: {outline}"));
    let nid = rollback["id"].as_i64().unwrap();

    let (status, body) = status_and_body(&srv, &format!("/nodes/{nid}?include=text")).await;
    assert_eq!((status, body["refusal"].as_str()), (StatusCode::FORBIDDEN, Some("gated_provider")), "{body}");
    let v = srv.get_ui_json(&format!("/nodes/{nid}?include=text")).await;
    assert!(v["text"].as_str().unwrap_or_default().contains("drain the settlement queue"), "{v}");
    // Metadata never gated, whatever the caller.
    srv.get_json(&format!("/nodes/{nid}?include=parent")).await;

    let (status, body) = status_and_body(&srv, &format!("/sections/{nid}/chunks")).await;
    assert_eq!((status, body["refusal"].as_str()), (StatusCode::FORBIDDEN, Some("gated_provider")), "{body}");
    let resp = srv.get_ui(&format!("/sections/{nid}/chunks")).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let chunks: Value = resp.json().await.expect("chunks");
    assert!(chunks.as_array().is_some_and(|a| !a.is_empty()), "{chunks}");

    let trusted = TestServer::start_with_env(&[("ANCHOR_CLOUD_TRUST", "1")]).await;
    let (sens_item, _) = synced_pair(&trusted, "gate-nodes-trusted").await;
    let outline = trusted.get_json(&format!("/items/{sens_item}/outline")).await;
    let nid = outline["sections"][0]["id"].as_i64().unwrap();
    let (status, _) = status_and_body(&trusted, &format!("/nodes/{nid}?include=text")).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = status_and_body(&trusted, &format!("/sections/{nid}/chunks")).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn tag_chunks_drop_sensitive_rows_for_agents() {
    // Both docs share vocabulary so one tag links chunks of both items.
    let srv = TestServer::start().await;
    let billing = "The billing migration settles the billing ledger nightly.\n".repeat(4);
    let sens = srv.mk_folder_source("tag-sens", &[("b.md", &billing)], Some("sensitive")).await;
    let open = srv.mk_folder_source("tag-open", &[("b.md", &billing)], None).await;
    let sens_sid = sens["id"].as_i64().unwrap();
    let open_sid = open["id"].as_i64().unwrap();
    for sid in [sens_sid, open_sid] {
        let job = srv.sync_and_wait(sid, "sync").await;
        assert_eq!(job["status"], "done", "{job}");
    }
    let tid = spanning_tag(&srv, sens_sid, open_sid).await.expect("a tag spanning both items");
    let before = gate_events(&srv);
    let agent: Value = srv.get_json(&format!("/tags/{tid}/chunks")).await;
    let agent_rows = agent.as_array().unwrap();
    assert!(!agent_rows.is_empty(), "open rows still list: {agent}");
    assert!(
        agent_rows.iter().all(|c| item_source(&srv, c["item_id"].as_i64().unwrap()) == Some(open_sid)),
        "sensitive rows drop: {agent}"
    );
    assert!(gate_events(&srv) > before, "the drop is audited");
    let ui: Value = srv.get_ui_json(&format!("/tags/{tid}/chunks")).await;
    assert!(ui.as_array().unwrap().len() > agent_rows.len(), "UI sees every row: {ui}");

    let trusted = TestServer::start_with_env(&[("ANCHOR_CLOUD_TRUST", "1")]).await;
    let sens = trusted.mk_folder_source("tag-sens", &[("b.md", &billing)], Some("sensitive")).await;
    let open = trusted.mk_folder_source("tag-open", &[("b.md", &billing)], None).await;
    let (tsens, topen) = (sens["id"].as_i64().unwrap(), open["id"].as_i64().unwrap());
    for sid in [tsens, topen] {
        trusted.sync_and_wait(sid, "sync").await;
    }
    let tid = spanning_tag(&trusted, tsens, topen).await.expect("a tag spanning both items");
    let chunks: Value = trusted.get_json(&format!("/tags/{tid}/chunks")).await;
    let sids: std::collections::HashSet<i64> = chunks
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|c| c["item_id"].as_i64().and_then(|iid| item_source(&trusted, iid)))
        .collect();
    assert!(sids.contains(&tsens) && sids.contains(&topen), "trust lists every row: {chunks}");
}

/// A tag whose chunks span both sources (read as the UI, which sees all).
async fn spanning_tag(srv: &TestServer, sens_sid: i64, open_sid: i64) -> Option<i64> {
    let tags: Value = srv.get_json("/tags").await;
    for tag in tags.as_array().unwrap() {
        let tid = tag["id"].as_i64().unwrap();
        let chunks: Value = srv.get_ui_json(&format!("/tags/{tid}/chunks")).await;
        let sids: std::collections::HashSet<i64> = chunks
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|c| c["item_id"].as_i64().and_then(|iid| item_source(srv, iid)))
            .collect();
        if sids.contains(&sens_sid) && sids.contains(&open_sid) {
            return Some(tid);
        }
    }
    None
}

fn item_source(srv: &TestServer, item_id: i64) -> Option<i64> {
    srv.db()
        .query_row("SELECT source_id FROM ingested_items WHERE id = ?1", [item_id], |r| r.get(0))
        .ok()
}

#[tokio::test]
async fn entity_reads_follow_the_caller_rule() {
    let srv = TestServer::start().await;
    let _ = synced_pair(&srv, "gate-entities").await;
    let all: Value = srv.get_ui_json("/entities").await;
    let sens_eid = all
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["summary"].as_str().unwrap_or_default().contains(SENSITIVE_WORD))
        .and_then(|e| e["id"].as_i64())
        .unwrap_or_else(|| panic!("no {SENSITIVE_WORD} entity: {all}"));

    for path in [format!("/entities/{sens_eid}"), format!("/entities/{sens_eid}/context")] {
        let (status, body) = status_and_body(&srv, &path).await;
        assert_eq!((status, body["refusal"].as_str()), (StatusCode::FORBIDDEN, Some("gated_provider")), "{path}: {body}");
    }
    let v = srv.get_ui_json(&format!("/entities/{sens_eid}")).await;
    assert!(v["window_text"].as_str().is_some_and(|w| !w.is_empty()) || v["summary"].as_str().is_some_and(|s| !s.is_empty()), "{v}");
    let v = srv.get_ui_json(&format!("/entities/{sens_eid}/context")).await;
    assert!(v["full_text"].as_str().is_some_and(|t| t.contains(SENSITIVE_WORD)), "{v}");

    // Lists drop the gated rows instead of refusing.
    let agent: Value = srv.get_json("/entities").await;
    let agent_rows = agent.as_array().unwrap();
    assert!(!agent_rows.is_empty(), "{agent}");
    assert!(
        agent_rows.iter().all(|e| !e["summary"].as_str().unwrap_or_default().contains(SENSITIVE_WORD)),
        "sensitive entities drop: {agent}"
    );
    assert!(
        all.as_array().unwrap().iter().any(|e| e["summary"].as_str().unwrap_or_default().contains(SENSITIVE_WORD)),
        "UI lists everything: {all}"
    );

    let trusted = TestServer::start_with_env(&[("ANCHOR_CLOUD_TRUST", "1")]).await;
    let _ = synced_pair(&trusted, "gate-entities-trusted").await;
    let all: Value = trusted.get_json("/entities").await;
    let sens_eid = all.as_array().unwrap().iter().find(|e| e["summary"].as_str().unwrap_or_default().contains(SENSITIVE_WORD)).and_then(|e| e["id"].as_i64()).unwrap();
    let (status, _) = status_and_body(&trusted, &format!("/entities/{sens_eid}")).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = status_and_body(&trusted, &format!("/entities/{sens_eid}/context")).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn related_and_review_lists_drop_sensitive_rows_for_agents() {
    let srv = TestServer::start().await;
    let (sens_item, open_item) = synced_pair(&srv, "gate-rel").await;
    // Planted links: a sensitive entity related to an open one, and a
    // low-confidence row on each side.
    let db = srv.db();
    let sens_eid: i64 = db
        .query_row(
            "SELECT e.id FROM entities e JOIN ingested_items i ON i.id = e.item_id WHERE i.source_id = (SELECT source_id FROM ingested_items WHERE id = ?1) LIMIT 1",
            [sens_item],
            |r| r.get(0),
        )
        .expect("sensitive entity");
    let open_eid: i64 = db
        .query_row(
            "SELECT e.id FROM entities e JOIN ingested_items i ON i.id = e.item_id WHERE i.source_id = (SELECT source_id FROM ingested_items WHERE id = ?1) LIMIT 1",
            [open_item],
            |r| r.get(0),
        )
        .expect("open entity");
    db.execute("INSERT INTO relationships (from_entity_id, to_entity_id) VALUES (?1, ?2)", rusqlite::params![open_eid, sens_eid])
        .expect("relationship");
    db.execute(
        "INSERT INTO entities (item_id, kind, summary, window_text, confidence, status) VALUES (?1, 'person', 'gate-rel-sensitive-low', 'low sealed text', 0.1, 'unverified')",
        [sens_item],
    )
    .expect("low sensitive");
    db.execute(
        "INSERT INTO entities (item_id, kind, summary, window_text, confidence, status) VALUES (?1, 'person', 'gate-rel-open-low', 'low open text', 0.1, 'unverified')",
        [open_item],
    )
    .expect("low open");
    drop(db);

    let agent: Value = srv.get_json(&format!("/entities/{open_eid}/related")).await;
    assert!(agent.as_array().unwrap().iter().all(|e| e["id"] != sens_eid), "sensitive related drops: {agent}");
    let ui: Value = srv.get_ui_json(&format!("/entities/{open_eid}/related")).await;
    assert!(ui.as_array().unwrap().iter().any(|e| e["id"] == sens_eid), "UI sees the link: {ui}");

    let agent: Value = srv.get_json("/review/low-confidence").await;
    assert!(
        agent.as_array().unwrap().iter().all(|e| e["summary"] != "gate-rel-sensitive-low"),
        "sensitive review rows drop: {agent}"
    );
    let ui: Value = srv.get_ui_json("/review/low-confidence").await;
    assert!(ui.as_array().unwrap().iter().any(|e| e["summary"] == "gate-rel-sensitive-low"), "UI reviews everything: {ui}");
}

#[tokio::test]
async fn ask_search_records_media_follow_the_caller_rule() {
    let srv = TestServer::start().await;
    let _ = synced_pair(&srv, "gate-answers").await;

    // Search: the agent loses the sensitive hit; the UI keeps it.
    let resp = srv.post("/qa/search", &json!({"query": SENSITIVE_WORD})).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let agent: Value = resp.json().await.expect("search");
    assert_eq!(agent["refusal"], "gated_provider", "{agent}");
    assert!(agent["blocked"].as_i64().unwrap_or(0) >= 1, "{agent}");
    let resp = srv.post_ui("/qa/search", &json!({"query": SENSITIVE_WORD})).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let ui: Value = resp.json().await.expect("search");
    assert!(ui["refusal"].is_null(), "{ui}");
    assert!(!ui["hits"].as_array().unwrap().is_empty(), "{ui}");

    // Ask: the agent is refused; the UI gets citations (no model key is
    // configured, so the answer falls back — the citations are the assert).
    let resp = srv.post("/qa", &json!({"question": format!("What about {SENSITIVE_WORD}?")})).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let agent: Value = resp.json().await.expect("ask");
    assert_eq!(agent["refusal"], "gated_provider", "{agent}");
    let resp = srv.post_ui("/qa", &json!({"question": format!("What about {SENSITIVE_WORD}?")})).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let ui: Value = resp.json().await.expect("ask");
    assert!(ui["refusal"].is_null(), "{ui}");
    assert!(!ui["citations"].as_array().unwrap().is_empty(), "{ui}");

    // Records: the agent loses the sensitive rows; the UI keeps them.
    for path in ["/decisions", "/actions"] {
        let agent: Value = srv.get_json(path).await;
        let ui: Value = srv.get_ui_json(path).await;
        let key = path.trim_start_matches('/');
        assert!(
            ui[key].as_array().unwrap().len() > agent[key].as_array().unwrap().len(),
            "UI keeps sensitive {key}: agent={agent} ui={ui}"
        );
    }

    // Media: the agent is refused the sensitive image; the UI downloads it.
    let png: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00,
        0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00,
        0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D,
        0xB4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];
    let source = srv
        .mk_folder_source("gate-img", &[("topo.md", "# Topo\n\nLook: ![Alt](img/i.png)\n")], Some("sensitive"))
        .await;
    let img_dir = srv.work.path().join("gate-img").join("img");
    std::fs::create_dir_all(&img_dir).expect("img dir");
    std::fs::write(img_dir.join("i.png"), png).expect("png");
    let job = srv.sync_and_wait(source["id"].as_i64().unwrap(), "sync").await;
    assert_eq!(job["status"], "done", "{job}");
    let uid: String = srv
        .db()
        .query_row("SELECT uid FROM assets LIMIT 1", [], |r| r.get(0))
        .expect("asset uid");
    let (status, body) = status_and_body(&srv, &format!("/media/{uid}")).await;
    assert_eq!((status, body["refusal"].as_str()), (StatusCode::FORBIDDEN, Some("gated_provider")), "{body}");
    let resp = srv.get_ui(&format!("/media/{uid}")).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.bytes().await.unwrap().as_ref(), png);
}

#[tokio::test]
async fn public_scopes_drop_effective_sensitive_whatever_the_provider_or_caller() {
    // Trusted provider: the public scope still drops the marked document.
    let srv = TestServer::start_with_env(&[("ANCHOR_CLOUD_TRUST", "1")]).await;
    let marked_doc = decision_doc("Public launch", &format!("The {SENSITIVE_WORD} launch is public news"), "Pam");
    let plain_doc = decision_doc("Office move", "We move offices in June", "Omar");
    let source = srv.mk_folder_source("pub-scope", &[("launch.md", &marked_doc), ("move.md", &plain_doc)], Some("public")).await;
    let sid = source["id"].as_i64().unwrap();
    let job = srv.sync_and_wait(sid, "sync").await;
    assert_eq!(job["status"], "done", "{job}");
    let marked: i64 = srv
        .db()
        .query_row("SELECT id FROM ingested_items WHERE source_id = ?1 AND external_id = 'launch.md'", [sid], |r| r.get(0))
        .unwrap();
    let plain: i64 = srv
        .db()
        .query_row("SELECT id FROM ingested_items WHERE source_id = ?1 AND external_id = 'move.md'", [sid], |r| r.get(0))
        .unwrap();
    assert_eq!(srv.patch(&format!("/items/{marked}"), &json!({"sensitive_override": true})).await.status(), StatusCode::OK);

    // Ask + search public scopes: the marked doc never surfaces.
    let resp = srv.post("/qa/public", &json!({"question": SENSITIVE_WORD})).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = resp.json().await.expect("public ask");
    assert!(body["citations"].as_array().unwrap().iter().all(|c| !cited(&c, "launch.md")), "{body}");
    let resp = srv.post("/qa/search", &json!({"query": SENSITIVE_WORD, "public_only": true})).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = resp.json().await.expect("public search");
    assert!(body["hits"].as_array().unwrap().iter().all(|h| h["path"].as_str().unwrap_or_default() != "launch.md"), "{body}");
    // The unmarked public doc still flows (control).
    let resp = srv.post("/qa/search", &json!({"query": "offices", "public_only": true})).await;
    let body: Value = resp.json().await.expect("control search");
    assert!(!body["hits"].as_array().unwrap().is_empty(), "{body}");

    // Records public scopes drop the marked decision.
    for path in ["/decisions?public_only=true", "/actions?public_only=true"] {
        let listed: Value = srv.get_json(path).await;
        let key = path.trim_start_matches('/').split('?').next().unwrap();
        assert!(
            listed[key].as_array().unwrap().iter().all(|r| r["provenance"]["path"] != "launch.md"),
            "{path}: {listed}"
        );
    }

    // Read surfaces: a marked item under public_only reads as not found.
    let (status, body) = status_and_body(&srv, &format!("/items/{marked}?public_only=true")).await;
    assert_eq!((status, body["detail"].as_str()), (StatusCode::NOT_FOUND, Some("Item not found")), "{body}");
    let (status, _) = status_and_body(&srv, &format!("/items/{marked}/outline?public_only=true")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let outline = srv.get_json(&format!("/items/{marked}/outline")).await;
    if let Some(node) = outline["sections"].as_array().unwrap().first() {
        let nid = node["id"].as_i64().unwrap();
        let (status, _) = status_and_body(&srv, &format!("/nodes/{nid}?include=text&public_only=true")).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }
    let listed: Value = srv.get_json("/items?public_only=true").await;
    assert!(listed["items"].as_array().unwrap().iter().all(|i| i["id"] != marked), "{listed}");
    assert!(listed["items"].as_array().unwrap().iter().any(|i| i["id"] == plain), "unmarked control: {listed}");

    // The UI header does not lift the public scope (the app's own public
    // preview must show the share-safe view).
    let resp = srv.post_ui("/qa/public", &json!({"question": SENSITIVE_WORD})).await;
    let body: Value = resp.json().await.expect("ui public ask");
    assert!(body["citations"].as_array().unwrap().iter().all(|c| !cited(&c, "launch.md")), "{body}");
    let (status, _) = status_and_body_ui(&srv, &format!("/items/{marked}?public_only=true")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

fn cited(citation: &Value, path: &str) -> bool {
    citation["source_ref"].as_str().is_some_and(|r| r.contains(path)) || citation["path"].as_str().is_some_and(|p| p.contains(path))
}

/// One `tools/call` over stdio against `anchorcore mcp` pointed at `srv`.
fn mcp_call(base: &str, tool: &str, args: Value) -> Value {
    let mut child = Command::new(env!("CARGO_BIN_EXE_anchorcore"))
        .arg("mcp")
        .env("ANCHOR_BACKEND_URL", base)
        .env("ANCHOR_SECRETS_NO_KEYRING", "1")
        .env_remove("ANCHOR_DATA_DIR")
        .env_remove("ANCHOR_DATABASE_URL")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn anchorcore mcp");
    let req = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": tool, "arguments": args}});
    child.stdin.take().expect("stdin").write_all(format!("{req}\n").as_bytes()).expect("write request");
    let out = child.wait_with_output().expect("mcp output");
    let line = String::from_utf8_lossy(&out.stdout).lines().next().unwrap_or_default().to_string();
    serde_json::from_str(&line).unwrap_or_else(|e| panic!("bad MCP reply {line:?}: {e}"))
}

fn tool_text(reply: &Value) -> String {
    reply["result"]["content"][0]["text"].as_str().unwrap_or_default().to_string()
}

#[tokio::test]
async fn mcp_reads_gate_sensitive_bytes() {
    let srv = TestServer::start().await;
    let (sens_item, open_item) = synced_pair(&srv, "gate-mcp").await;
    let base = srv.url("");
    let call = |tool: &'static str, args: Value| {
        let base = base.clone();
        tokio::task::spawn_blocking(move || mcp_call(&base, tool, args))
    };

    // Whole-file get_item without lines (R36.19, absorbed): gated.
    let reply = call("get_item", json!({"item_id": sens_item})).await.unwrap();
    assert!(tool_text(&reply).contains("gated_provider"), "{reply}");
    let reply = call("get_item", json!({"item_id": sens_item, "lines": "1"})).await.unwrap();
    assert!(tool_text(&reply).contains("gated_provider"), "{reply}");
    let reply = call("get_item", json!({"item_id": open_item})).await.unwrap();
    assert_eq!(reply["result"]["isError"], false, "{reply}");

    let all: Value = srv.get_ui_json("/entities").await;
    let sens_eid = all
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["summary"].as_str().unwrap_or_default().contains(SENSITIVE_WORD))
        .and_then(|e| e["id"].as_i64())
        .expect("sensitive entity");
    let reply = call("get_entity", json!({"entity_id": sens_eid})).await.unwrap();
    assert!(tool_text(&reply).contains("gated_provider"), "{reply}");
}
