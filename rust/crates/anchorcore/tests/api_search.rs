//! API contract: agent surface — /qa/search, entities, records (black-box).
//!
//! Ports the REST assertions behind the retired `test_mcp.py` (ignored in
//! CI, but its search/entity/project expectations are the agent contract),
//! the B26 window-context test reframed for record fixtures, the B7 source
//! editing tests, and the decisions/actions listing shapes.

mod common;

use common::{TestServer, action_doc, decision_doc};
use reqwest::StatusCode;
use serde_json::{Value, json};

#[tokio::test]
async fn search_returns_ranked_hits() {
    let srv = TestServer::start().await;
    let alpha = srv
        .mk_folder_source("alpha", &[("doc.md", "# Alpha\nDecision: the alpha team owns the ACME movement rules.\n")], None)
        .await;
    let beta = srv
        .mk_folder_source("beta", &[("doc.md", "# Beta\nDecision: the beta team ships the DRB transport spec.\n")], None)
        .await;
    srv.sync_and_wait(alpha["id"].as_i64().unwrap(), "sync").await;
    srv.sync_and_wait(beta["id"].as_i64().unwrap(), "sync").await;

    let resp = srv.post("/qa/search", &json!({"query": "movement rules"})).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = resp.json().await.expect("search");
    let hits = body["hits"].as_array().unwrap();
    assert!(!hits.is_empty(), "search should return hits: {body}");
    let scores: Vec<f64> = hits.iter().filter_map(|h| h["score"].as_f64()).collect();
    assert!(scores.windows(2).all(|w| w[0] >= w[1]), "hits must be ranked desc: {scores:?}");
    let sources: Vec<i64> = hits.iter().filter_map(|h| h["source_id"].as_i64()).collect();
    assert!(sources.contains(&alpha["id"].as_i64().unwrap()), "{sources:?}");
    assert!(!sources.contains(&beta["id"].as_i64().unwrap()), "alpha query must not hit beta: {sources:?}");
    let hit = &hits[0];
    for key in ["source_ref", "kind", "score", "provenance"] {
        assert!(hit.get(key).is_some(), "hit lacks {key}: {hit}");
    }
}

#[tokio::test]
async fn search_scopes_to_project() {
    let srv = TestServer::start().await;
    let a = srv
        .mk_folder_source("scoped-a", &[("doc.md", "# ScopedA\nDecision: scoped a owns the VELOCITY budget.\n")], None)
        .await;
    let b = srv
        .mk_folder_source("scoped-b", &[("doc.md", "# ScopedB\nDecision: scoped b owns the TORQUE budget.\n")], None)
        .await;
    srv.sync_and_wait(a["id"].as_i64().unwrap(), "sync").await;
    srv.sync_and_wait(b["id"].as_i64().unwrap(), "sync").await;
    let project: Value = srv
        .post("/projects", &json!({"name": "velo", "source_ids": [a["id"]]}))
        .await
        .json()
        .await
        .expect("project");

    let resp = srv
        .post("/qa/search", &json!({"query": "budget", "k": 10, "project_id": project["id"]}))
        .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = resp.json().await.expect("search");
    let hits = body["hits"].as_array().unwrap();
    assert!(!hits.is_empty(), "{body}");
    assert!(hits.iter().all(|h| h["source_id"] == a["id"]), "{hits:?}");
    assert!(!hits.iter().any(|h| h["source_id"] == b["id"]), "{hits:?}");
}

#[tokio::test]
async fn search_content_opt_in() {
    let srv = TestServer::start().await;
    let doc = decision_doc("Horizon rollout", "The probe team owns the HORIZON rollout plan", "Horst");
    let source = srv.mk_folder_source("probe", &[("doc.md", &doc)], None).await;
    srv.sync_and_wait(source["id"].as_i64().unwrap(), "sync").await;

    let compact: Value = srv
        .post("/qa/search", &json!({"query": "HORIZON rollout"}))
        .await
        .json()
        .await
        .expect("search");
    let hits = compact["hits"].as_array().unwrap();
    assert!(!hits.is_empty(), "{compact}");
    assert!(hits.iter().all(|h| h["content"].as_str().unwrap_or_default().is_empty()), "content must be opt-in: {hits:?}");

    let full: Value = srv
        .post("/qa/search", &json!({"query": "HORIZON rollout", "include_content": true}))
        .await
        .json()
        .await
        .expect("search");
    let full_hits = full["hits"].as_array().unwrap();
    assert!(!full_hits.is_empty(), "{full}");
    assert!(full_hits.iter().all(|h| !h["content"].as_str().unwrap_or_default().is_empty()), "{full_hits:?}");
}

#[tokio::test]
async fn get_entity_shape_and_404() {
    let srv = TestServer::start().await;
    let doc = decision_doc("Horizon rollout", "The probe team owns the HORIZON rollout plan", "Horst");
    let source = srv.mk_folder_source("probe", &[("doc.md", &doc)], None).await;
    srv.sync_and_wait(source["id"].as_i64().unwrap(), "sync").await;

    let entities: Value = srv.get("/entities").await.json().await.expect("entities");
    let eid = entities[0]["id"].as_i64().unwrap();
    let entity: Value = srv.get(&format!("/entities/{eid}")).await.json().await.expect("entity");
    assert_eq!(entity["id"], eid, "{entity}");
    assert!(!entity["kind"].as_str().unwrap_or_default().is_empty(), "{entity}");
    assert!(entity.get("window_text").is_some(), "{entity}");
    assert!(entity.get("chunk_id").is_some(), "{entity}");

    assert_eq!(srv.get("/entities/999999").await.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn get_source_shape_and_list() {
    let srv = TestServer::start().await;
    let doc = decision_doc("Horizon rollout", "The probe team owns the HORIZON rollout plan", "Horst");
    let source = srv.mk_folder_source("Probe", &[("doc.md", &doc)], None).await;
    let sid = source["id"].as_i64().unwrap();

    let fetched: Value = srv.get(&format!("/sources/{sid}")).await.json().await.expect("source");
    assert_eq!(fetched["name"], "Probe", "{fetched}");
    assert_eq!(fetched["connector"], "folder", "{fetched}");

    let sources: Value = srv.get("/sources").await.json().await.expect("sources");
    assert!(sources.as_array().unwrap().iter().any(|s| s["name"] == "Probe"), "{sources}");
}

#[tokio::test]
async fn decisions_and_actions_listings() {
    let srv = TestServer::start().await;
    let d = decision_doc("Billing provider", "We approved moving to the new billing provider", "Sarah");
    let a = action_doc("re-baseline test coverage on new hosts and publish corrected number", "Priya");
    let source = srv.mk_folder_source("records", &[("d.md", &d), ("a.md", &a)], None).await;
    let job = srv.sync_and_wait(source["id"].as_i64().unwrap(), "sync").await;
    assert_eq!(job["status"], "done", "{job}");

    let decisions: Value = srv.get("/decisions").await.json().await.expect("decisions");
    assert!(
        decisions["decisions"].as_array().unwrap().iter().any(|r| {
            r["summary"].as_str().unwrap_or_default().contains("billing provider")
        }),
        "{decisions}"
    );
    let actions: Value = srv.get("/actions").await.json().await.expect("actions");
    assert!(
        actions["actions"].as_array().unwrap().iter().any(|r| {
            r["summary"].as_str().unwrap_or_default().contains("re-baseline test coverage")
                || r["task"].as_str().unwrap_or_default().contains("re-baseline test coverage")
        }),
        "{actions}"
    );
}

#[tokio::test]
async fn entity_window_context_and_reclassify_skip() {
    let srv = TestServer::start().await;
    let doc = decision_doc(
        "Preliminary advice",
        "The issuing bank must send a preliminary advice message",
        "Ruth",
    );
    let source = srv.mk_folder_source("b26", &[("rules.md", &doc)], None).await;
    let sid = source["id"].as_i64().unwrap();
    let job = srv.sync_and_wait(sid, "sync").await;
    assert_eq!(job["status"], "done", "{job}");

    let entities: Value = srv.get("/entities").await.json().await.expect("entities");
    let list = entities.as_array().unwrap();
    assert!(!list.is_empty(), "expected entities");
    assert!(list.iter().all(|e| !e["window_text"].as_str().unwrap_or_default().is_empty()), "window_text must be set: {list:?}");

    let hashes: String = srv
        .db()
        .query_row("SELECT window_hashes FROM ingested_items WHERE source_id = ?1", [sid], |r| r.get(0))
        .expect("hashes");
    assert_ne!(hashes, "{}", "window hashes must be persisted");
    let before = list.len();

    // Reclassify skips unchanged windows → no new entities.
    let job2 = srv.sync_and_wait(sid, "reclassify").await;
    assert_eq!(job2["status"], "done", "{job2}");
    let entities2: Value = srv.get("/entities").await.json().await.expect("entities");
    assert_eq!(entities2.as_array().unwrap().len(), before, "unchanged windows must not re-classify");

    let low: Value = srv.get("/review/low-confidence").await.json().await.expect("low");
    assert!(low.as_array().unwrap().iter().all(|e| e.get("window_text").is_some()), "review must expose window_text: {low}");
}

#[tokio::test]
async fn update_name_and_enabled() {
    let srv = TestServer::start().await;
    let doc = decision_doc("Keep fresh", "We decided to keep this source fresh", "Rex");
    let source = srv.mk_folder_source("old-name", &[("d.md", &doc)], None).await;
    let sid = source["id"].as_i64().unwrap();

    let resp = srv.put(&format!("/sources/{sid}"), &json!({"name": "new-name", "enabled": false})).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let updated: Value = resp.json().await.expect("update");
    assert_eq!(updated["name"], "new-name", "{updated}");
    assert_eq!(updated["enabled"], false, "{updated}");

    let partial: Value = srv
        .put(&format!("/sources/{sid}"), &json!({"name": "renamed-again"}))
        .await
        .json()
        .await
        .expect("partial");
    assert_eq!(partial["name"], "renamed-again", "{partial}");
    assert_eq!(partial["enabled"], false, "{partial}");

    assert_eq!(
        srv.put(&format!("/sources/{sid}"), &json!({"name": "  "})).await.status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        srv.put("/sources/999999", &json!({"name": "x"})).await.status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn update_folder_config_and_resync() {
    let srv = TestServer::start().await;
    let doc = decision_doc("Keep fresh", "We decided to keep this source fresh", "Rex");
    let source = srv.mk_folder_source("folder-edit", &[("d.md", &doc)], None).await;
    let sid = source["id"].as_i64().unwrap();
    let config: Value = srv
        .get(&format!("/sources/{sid}/config"))
        .await
        .json()
        .await
        .expect("config");
    assert!(config["path"].as_str().unwrap_or_default().contains("folder-edit"), "{config}");

    let other = srv.work.path().join("other");
    std::fs::create_dir_all(&other).expect("other dir");
    let b = decision_doc("Point here", "We decided to point here now", "Uma");
    std::fs::write(other.join("b.md"), &b).expect("file");
    let resp = srv
        .put(&format!("/sources/{sid}"), &json!({"config": {"path": other.to_string_lossy()}}))
        .await;
    assert_eq!(resp.status(), StatusCode::OK, "PUT config");
    let config2: Value = srv
        .get(&format!("/sources/{sid}/config"))
        .await
        .json()
        .await
        .expect("config");
    assert_eq!(config2["path"].as_str().unwrap(), other.to_string_lossy(), "{config2}");

    let job = srv.sync_and_wait(sid, "sync").await;
    assert_eq!(job["status"], "done", "{job}");
    assert_eq!(job["result"]["items"], 1, "{job}");
}

async fn source_config(srv: &TestServer, sid: i64) -> Value {
    srv.get(&format!("/sources/{sid}/config")).await.json().await.expect("config")
}

#[tokio::test]
async fn secret_placeholder_semantics() {
    // Observable half of the keychain-backed contract: `***set***` leaves the
    // stored value alone, explicit "" clears it, new values mask on read.
    let srv = TestServer::start().await;
    let created: Value = srv
        .post(
            "/sources",
            &json!({
                "connector": "jira",
                "name": "jira-secret",
                "config": {"base_url": "https://x.atlassian.net", "email": "a@b.c", "token": "orig-token"},
            }),
        )
        .await
        .json()
        .await
        .expect("source");
    let sid = created["id"].as_i64().unwrap();

    assert_eq!(source_config(&srv, sid).await["token"], "***set***");

    // The masked placeholder keeps the stored value (still set afterwards).
    let keep = srv
        .put(&format!("/sources/{sid}"), &json!({"config": {"base_url": "https://y.atlassian.net", "token": "***set***"}}))
        .await;
    assert_eq!(keep.status(), StatusCode::OK, "PUT placeholder");
    let kept = source_config(&srv, sid).await;
    assert_eq!(kept["token"], "***set***", "{kept}");
    assert_eq!(kept["base_url"], "https://y.atlassian.net", "{kept}");

    // A real value replaces it (still masked on read, never plaintext).
    srv.put(&format!("/sources/{sid}"), &json!({"config": {"token": "new-token"}})).await;
    let replaced = source_config(&srv, sid).await;
    assert_eq!(replaced["token"], "***set***", "{replaced}");
    assert!(!replaced.to_string().contains("new-token"), "{replaced}");

    // Clearing the field removes the stored secret.
    srv.put(&format!("/sources/{sid}"), &json!({"config": {"token": ""}})).await;
    let cleared = source_config(&srv, sid).await;
    assert!(cleared.get("token").is_none_or(|v| v != "***set***"), "{cleared}");

    // New secret fields mask by single source of truth (no per-field wiring).
    let auto: Value = srv
        .post(
            "/sources",
            &json!({
                "connector": "jira",
                "name": "auto-mask",
                "config": {"base_url": "https://x.atlassian.net", "client_secret": "shh-secret"},
            }),
        )
        .await
        .json()
        .await
        .expect("source");
    let masked = source_config(&srv, auto["id"].as_i64().unwrap()).await;
    assert_eq!(masked["client_secret"], "***set***", "{masked}");
    assert!(!masked.to_string().contains("shh-secret"), "{masked}");
}

#[tokio::test]
async fn diagram_files_index_as_flow_text() {
    // R46.2 end to end: folder sync extracts draw.io + Excalidraw files to
    // rendered markdown, and a "what happens after" question finds the edge.
    let drawio = r#"<mxfile><diagram name="Fulfilment"><mxGraphModel><root>
<mxCell id="0"/><mxCell id="1" parent="0"/>
<mxCell id="a" value="Warehouse picks parcel" vertex="1" parent="1"><mxGeometry x="0" y="0" width="120" height="60" as="geometry"/></mxCell>
<mxCell id="b" value="Courier collects parcel" vertex="1" parent="1"><mxGeometry x="200" y="0" width="120" height="60" as="geometry"/></mxCell>
<mxCell id="e" value="before noon" edge="1" parent="1" source="a" target="b"><mxGeometry relative="1" as="geometry"/></mxCell>
</root></mxGraphModel></diagram></mxfile>"#;
    let scene = r#"{"type":"excalidraw","elements":[
{"id":"r","type":"rectangle","x":0,"y":0,"height":40},{"id":"rt","type":"text","text":"Quarterly roadmap review","containerId":"r","x":0,"y":0},
{"id":"s","type":"rectangle","x":300,"y":0,"height":40},{"id":"st","type":"text","text":"Publish roadmap","containerId":"s","x":0,"y":0},
{"id":"arr","type":"arrow","x":0,"y":0,"startBinding":{"elementId":"r"},"endBinding":{"elementId":"s"}}]}"#;
    let srv = TestServer::start().await;
    let source = srv
        .mk_folder_source(
            "flows",
            &[("fulfilment.drawio", drawio), ("roadmap.excalidraw.json", scene), ("icon.svg", "<svg><path d=\"M0 0\"/></svg>")],
            None,
        )
        .await;
    let job = srv.sync_and_wait(source["id"].as_i64().unwrap(), "sync").await;
    assert_eq!(job["status"], "done", "{job}");

    let cases = [
        ("what happens after the warehouse picks the parcel", "fulfilment.drawio", "Warehouse picks parcel → Courier collects parcel: before noon"),
        ("what follows the quarterly roadmap review", "roadmap.excalidraw.json", "Quarterly roadmap review → Publish roadmap"),
    ];
    for (query, file, edge) in cases {
        let resp = srv.post("/qa/search", &json!({"query": query, "include_content": true})).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body: Value = resp.json().await.expect("search");
        let hits = body["hits"].as_array().expect("hits");
        let hit = hits
            .iter()
            .find(|h| h["content"].as_str().is_some_and(|c| c.contains(edge)))
            .unwrap_or_else(|| panic!("no hit carries {edge:?} for {query:?}: {body}"));
        assert!(hit.to_string().contains(file), "hit should cite {file}: {hit}");
    }
}

#[tokio::test]
async fn miro_source_is_config_only_and_masks_its_token() {
    // R46.5: create never calls Miro (config-only, like Jira); the access
    // token lives in the secret store and reads back masked.
    let srv = TestServer::start().await;
    let created: Value = srv
        .post(
            "/sources",
            &json!({"connector": "miro", "name": "whiteboards", "config": {"token": "miro-secret-token", "boards": "uXjVO1abc="}}),
        )
        .await
        .json()
        .await
        .expect("source");
    let sid = created["id"].as_i64().expect("created");
    let config = source_config(&srv, sid).await;
    assert_eq!(config["token"], "***set***", "{config}");
    assert_eq!(config["boards"], "uXjVO1abc=", "{config}");
    assert!(!config.to_string().contains("miro-secret-token"), "{config}");
    assert!(!created.to_string().contains("miro-secret-token"), "{created}");
}
