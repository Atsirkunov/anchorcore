//! API contract: image assets (R46.9) — `GET /media/:uid`, `images` on
//! search hits and agent citations, `assets` on item detail (black-box).

mod common;

use common::TestServer;
use reqwest::StatusCode;
use serde_json::{Value, json};

/// 1×1 transparent PNG.
const PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00,
    0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0D, 0x49,
    0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00,
    0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
];

const DOC: &str = "# Topology\n\nThe ingress topology diagram: ![Ingress topology](img/ingress.png \"Ingress layout\")\n";

/// Folder source with one markdown doc embedding one PNG, synced.
async fn synced_image_source(srv: &TestServer, name: &str, label: Option<&str>) -> Value {
    let source = srv.mk_folder_source(name, &[("topology.md", DOC)], label).await;
    let img_dir = srv.work.path().join(name).join("img");
    std::fs::create_dir_all(&img_dir).expect("img dir");
    std::fs::write(img_dir.join("ingress.png"), PNG).expect("png");
    let job = srv.sync_and_wait(source["id"].as_i64().unwrap(), "sync").await;
    assert_eq!(job["status"], "done", "{job}");
    source
}

async fn search(srv: &TestServer, body: Value) -> Value {
    let resp = srv.post("/qa/search", &body).await;
    assert_eq!(resp.status(), StatusCode::OK);
    resp.json().await.expect("search json")
}

#[tokio::test]
async fn media_served_from_db_after_folder_is_deleted() {
    let srv = TestServer::start().await;
    synced_image_source(&srv, "topo", None).await;

    // Default shape: no `images` key at all.
    let plain = search(&srv, json!({"query": "ingress topology"})).await;
    let hits = plain["hits"].as_array().expect("hits");
    assert!(!hits.is_empty(), "{plain}");
    assert!(hits.iter().all(|h| h.get("images").is_none()), "images must be opt-in: {plain}");

    let body = search(&srv, json!({"query": "ingress topology", "fields": "summary,images"})).await;
    let hit = body["hits"]
        .as_array()
        .unwrap()
        .iter()
        .find(|h| h["images"].as_array().is_some_and(|a| !a.is_empty()))
        .unwrap_or_else(|| panic!("no hit carries images: {body}"))
        .clone();
    let image = &hit["images"][0];
    assert_eq!(image["alt"], "Ingress topology");
    assert_eq!(image["caption"], "Ingress layout");
    assert_eq!((image["mime"].as_str(), image["w"].as_i64(), image["h"].as_i64()), (Some("image/png"), Some(1), Some(1)));
    let url = image["url"].as_str().expect("url").to_string();
    assert!(url.starts_with("/media/"), "{url}");

    // Item drill-down lists the embed and the file asset.
    let item: Value = srv.get(&format!("/items/{}", hit["item_id"])).await.json().await.expect("item");
    assert!(item["assets"].as_array().is_some_and(|a| a.iter().any(|x| x["asset_id"] == image["asset_id"])), "{item}");

    let fetch = |srv: &TestServer, url: String| {
        let req = srv.client.get(srv.url(&url));
        async move { req.send().await.expect("media request") }
    };
    let resp = fetch(&srv, url.clone()).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let headers = resp.headers().clone();
    assert_eq!(headers["content-type"], "image/png");
    assert_eq!(headers["x-content-type-options"], "nosniff");
    assert!(headers["content-security-policy"].to_str().unwrap().contains("sandbox"));
    assert_eq!(resp.bytes().await.unwrap().as_ref(), PNG);

    std::fs::remove_dir_all(srv.work.path().join("topo")).expect("delete folder");
    let again = fetch(&srv, url.clone()).await;
    assert_eq!(again.status(), StatusCode::OK, "served from the DB, not the disk");
    assert_eq!(again.bytes().await.unwrap().as_ref(), PNG);

    let missing = fetch(&srv, "/media/0123456789abcdef0123456789abcdef".to_string()).await;
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);

    // R46.10 agent form: JSON + base64 for harnesses that can only read JSON.
    let agent: Value = fetch(&srv, format!("{url}?format=json")).await.json().await.expect("agent json");
    assert_eq!(agent["mime"], "image/png");
    assert_eq!((agent["w"].as_i64(), agent["size"].as_u64()), (Some(1), Some(PNG.len() as u64)));
    assert_eq!(agent["data_base64"], b64(PNG), "{agent}");
}

fn b64(bytes: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in bytes.chunks(3) {
        let n = (u32::from(c[0]) << 16) | (u32::from(*c.get(1).unwrap_or(&0)) << 8) | u32::from(*c.get(2).unwrap_or(&0));
        for i in 0..4 {
            out.push(if i <= c.len() { T[(n >> (18 - 6 * i) & 63) as usize] as char } else { '=' });
        }
    }
    out
}

#[tokio::test]
async fn agent_form_refuses_images_over_its_cap_with_the_raw_url() {
    // Stored (under asset_max_bytes) but over the 1.5 MB agent cap: the JSON
    // form says so with size + url; the raw download still works.
    let srv = TestServer::start().await;
    let source = srv.mk_folder_source("big", &[("big.md", "# Big\n\nA wide panorama: ![Panorama](pano.png)\n")], None).await;
    let mut pano = PNG.to_vec();
    pano.resize(1_700_000, 0);
    std::fs::write(srv.work.path().join("big").join("pano.png"), &pano).expect("png");
    srv.sync_and_wait(source["id"].as_i64().unwrap(), "sync").await;
    let body = search(&srv, json!({"query": "wide panorama", "fields": "images"})).await;
    let url = body["hits"][0]["images"][0]["url"].as_str().unwrap_or_else(|| panic!("{body}")).to_string();
    let resp = srv.client.get(srv.url(&format!("{url}?format=json"))).send().await.expect("agent form");
    assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let refusal: Value = resp.json().await.expect("413 json");
    assert_eq!((refusal["size"].as_u64(), refusal["url"].as_str()), (Some(1_700_000), Some(url.as_str())), "{refusal}");
    let raw = srv.client.get(srv.url(&url)).send().await.expect("raw");
    assert_eq!(raw.status(), StatusCode::OK);
    assert_eq!(raw.bytes().await.unwrap().len(), 1_700_000);
}

#[tokio::test]
async fn media_gated_for_sensitive_source_under_untrusted_provider() {
    let srv = TestServer::start().await;
    let source = synced_image_source(&srv, "secret-topo", Some("sensitive")).await;
    let items: Value = srv.get(&format!("/items?source_id={}", source["id"])).await.json().await.expect("items");
    let doc = items["items"]
        .as_array()
        .expect("items list")
        .iter()
        .find(|i| i["path"] == "topology.md")
        .unwrap_or_else(|| panic!("{items}"))
        .clone();
    let item: Value = srv.get(&format!("/items/{}", doc["id"])).await.json().await.expect("item");
    let url = item["assets"][0]["url"].as_str().expect("asset url").to_string();

    for (k, v) in [("answer_base_url", "https://192.0.2.1"), ("answer_api_key", "sk-test")] {
        assert_eq!(srv.put("/settings", &json!({ k: v })).await.status(), StatusCode::OK, "set {k}");
    }
    for target in [url.clone(), format!("{url}?format=json")] {
        let resp = srv.client.get(srv.url(&target)).send().await.expect("media");
        assert_eq!(resp.status(), StatusCode::FORBIDDEN, "{target}");
        let body: Value = resp.json().await.expect("refusal json");
        assert_eq!(body["refusal"], "gated_provider", "{body}");
    }
}

#[tokio::test]
async fn agent_compose_citations_carry_images() {
    let srv = TestServer::start().await;
    synced_image_source(&srv, "agent-topo", None).await;
    assert_eq!(srv.put("/settings", &json!({"llm_mode": "agent"})).await.status(), StatusCode::OK);
    let resp = srv.post("/qa", &json!({"question": "Where is the ingress topology diagram?"})).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = resp.json().await.expect("qa");
    assert_eq!(body["mode"], "agent_compose", "{body}");
    let images: Vec<&Value> = body["citations"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|c| c["images"].as_array())
        .flatten()
        .collect();
    assert!(images.iter().any(|i| i["alt"] == "Ingress topology" && i["url"].is_string()), "{body}");
}

// --- R46.12: Unplaced images ---

const GUIDE: &str = "# Ledger guide\n\n## Payout\n\nThe ledger closes every night.\n";

/// Folder source with one doc and one image file nothing links: the image
/// is unplaced (attribution `none`). Returns (source id, doc item id, asset).
async fn unplaced_source(srv: &TestServer, name: &str, label: Option<&str>) -> (i64, i64, Value) {
    let source = srv.mk_folder_source(name, &[("guide.md", GUIDE)], label).await;
    std::fs::write(srv.work.path().join(name).join("flow.png"), PNG).expect("png");
    let sid = source["id"].as_i64().unwrap();
    assert_eq!(srv.sync_and_wait(sid, "sync").await["status"], "done");
    let listed = srv.get_json(&format!("/media?attribution=none&source_id={sid}")).await;
    assert_eq!(listed["total"], 1, "{listed}");
    let asset = listed["items"][0].clone();
    assert_eq!((asset["path"].as_str(), asset["origin"].as_str()), (Some("flow.png"), Some("file")), "{asset}");
    let items = srv.get_json(&format!("/items?source_id={sid}")).await;
    let doc = items["items"].as_array().unwrap().iter().find(|i| i["path"] == "guide.md").expect("doc item")["id"].as_i64().unwrap();
    (sid, doc, asset)
}

/// Counts every connection: no route under test may reach a model.
async fn chat_counter() -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = calls.clone();
    tokio::spawn(async move {
        while let Ok((_sock, _)) = listener.accept().await {
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    });
    (base, calls)
}

#[tokio::test]
async fn unplaced_image_caption_answers_and_survives_resync() {
    let srv = TestServer::start().await;
    let (base, calls) = chat_counter().await;
    assert_eq!(srv.put("/settings", &json!({"answer_base_url": base})).await.status(), StatusCode::OK);
    let (sid, doc, asset) = unplaced_source(&srv, "ledger", None).await;
    let uid = asset["asset_id"].as_str().unwrap().to_string();

    // Validation happens before any write; unknown uid is a 404.
    let bad = srv.patch(&format!("/media/{uid}"), &json!({"target": {"item_id": doc, "entity_id": 1}})).await;
    assert_eq!(bad.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let missing = srv.patch("/media/nope", &json!({"caption": "x"})).await;
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    assert_eq!(srv.get("/media?attribution=bogus").await.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let resp = srv.patch(&format!("/media/{uid}"), &json!({"caption": "Quorvane payout flow: batches clear at midnight", "target": {"item_id": doc}})).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let row: Value = resp.json().await.unwrap();
    assert_eq!((row["attribution"].as_str(), row["target"]["item_id"].as_i64()), (Some("manual"), Some(doc)), "{row}");
    assert_eq!(srv.get_json("/media").await["total"], 0, "placed images leave the unplaced list");

    async fn search_caption(srv: &TestServer) -> Value {
        search(srv, json!({"query": "quorvane payout flow", "fields": "summary,images"})).await
    }
    let body = search_caption(&srv).await;
    let hit = body["hits"].as_array().unwrap().iter().find(|h| h["kind"] == "caption").unwrap_or_else(|| panic!("no caption hit: {body}")).clone();
    assert!(hit["path"].as_str().unwrap().ends_with("caption by a person"), "{hit}");
    assert_eq!(hit["item_id"].as_i64(), Some(doc), "caption sits on its target");
    assert!(hit["images"].as_array().unwrap().iter().any(|i| i["asset_id"] == uid.as_str()), "{hit}");

    // Edit the target doc and re-sync: caption + target survive.
    std::fs::write(srv.work.path().join("ledger").join("guide.md"), format!("{GUIDE}\n## Audit\n\nAuditors read the ledger.\n")).unwrap();
    assert_eq!(srv.sync_and_wait(sid, "sync").await["status"], "done");
    let again = search_caption(&srv).await;
    assert_eq!(again["hits"].as_array().unwrap().iter().filter(|h| h["kind"] == "caption").count(), 1, "{again}");
    let kept = srv.get_json("/media?attribution=manual").await;
    assert_eq!(kept["items"][0]["manual_caption"], "Quorvane payout flow: batches clear at midnight", "{kept}");

    // Ignore hides it from answers without deleting it; un-ignore + clear
    // hands it back to the unplaced list.
    let ignored: Value = srv.patch(&format!("/media/{uid}"), &json!({"ignore": true})).await.json().await.unwrap();
    assert_eq!(ignored["attribution"], "ignored");
    assert!(search_caption(&srv).await["hits"].as_array().unwrap().iter().all(|h| h["kind"] != "caption"));
    assert_eq!(srv.get_json("/media?attribution=ignored").await["total"], 1);
    let cleared: Value = srv.patch(&format!("/media/{uid}"), &json!({"ignore": false, "caption": "", "target": null})).await.json().await.unwrap();
    assert_eq!((cleared["attribution"].as_str(), cleared["target"].is_null()), (Some("none"), true), "{cleared}");
    assert_eq!(srv.get_json("/media").await["total"], 1);

    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0, "no chat call anywhere in the unplaced flow");
}

#[tokio::test]
async fn caption_in_sensitive_source_is_gated_like_its_source() {
    let srv = TestServer::start().await;
    let (_, _, asset) = unplaced_source(&srv, "secret-ledger", Some("sensitive")).await;
    let uid = asset["asset_id"].as_str().unwrap();
    let resp = srv.patch(&format!("/media/{uid}"), &json!({"caption": "Brennick vault rotation diagram"})).await;
    assert_eq!(resp.status(), StatusCode::OK);
    for (k, v) in [("answer_base_url", "https://192.0.2.1"), ("answer_api_key", "sk-test")] {
        assert_eq!(srv.put("/settings", &json!({ k: v })).await.status(), StatusCode::OK, "set {k}");
    }
    let body = search(&srv, json!({"query": "brennick vault rotation"})).await;
    assert!(body["hits"].as_array().unwrap().is_empty(), "{body}");
    assert_eq!((body["blocked"].as_u64(), body["refusal"].as_str()), (Some(1), Some("gated_provider")), "{body}");
}
