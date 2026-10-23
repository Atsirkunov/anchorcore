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
