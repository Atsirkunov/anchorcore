//! R45.4: `/system/embed-model` (black-box). Without weights the app stays
//! on the HTTP embedder; the download is an explicit POST that reports its
//! progress (here against a dead mirror, so it fails cleanly and offline).

mod common;

use std::time::{Duration, Instant};

use common::TestServer;
use serde_json::json;

#[tokio::test]
async fn embed_model_status_and_failed_download() {
    let srv = TestServer::start_with_env(&[("ANCHOR_MODELS_BASE_URL", "http://127.0.0.1:1")]).await;
    let st = srv.get_json("/system/embed-model").await;
    assert_eq!(st["model"], "nomic-embed-text-v1.5");
    assert_eq!(st["setting"], "auto");
    assert_eq!(st["active"], "ollama", "auto stays on HTTP until the model is ready");
    assert_eq!(st["ready"], false);
    assert!(st["download_bytes"].as_u64().unwrap() > 500_000_000);
    assert!(st["dir"].as_str().unwrap().ends_with("nomic-embed-text-v1.5"));

    let resp = srv.post("/system/embed-model/download", &json!({})).await;
    assert_eq!(resp.status().as_u16(), 202);
    let deadline = Instant::now() + Duration::from_secs(20);
    let progress = loop {
        let p = srv.get_json("/system/embed-model").await["progress"].clone();
        if p["phase"] == "failed" || Instant::now() > deadline {
            break p;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    assert_eq!(progress["phase"], "failed", "{progress}");
    assert!(progress["error"].as_str().unwrap().contains("config.json"), "{progress}");
    assert_eq!(srv.get_json("/system/embed-model").await["active"], "ollama");
}

#[tokio::test]
async fn embed_provider_setting_is_validated() {
    let srv = TestServer::start().await;
    let bad = srv.put("/settings", &json!({"embed_provider": "gpu"})).await;
    assert_eq!(bad.status().as_u16(), 422, "unknown provider is rejected");
    let ok = srv.put("/settings", &json!({"embed_provider": "builtin"})).await;
    assert!(ok.status().is_success());
    let st = srv.get_json("/system/embed-model").await;
    assert_eq!(st["setting"], "builtin");
    assert_eq!(st["active"], "builtin", "explicit builtin even before the download");
}
