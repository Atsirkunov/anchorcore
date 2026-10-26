//! R20.4: `runtime.json` discovery (black-box). The server advertises its
//! port in the data dir; `anchorcore mcp --data-dir` finds it with no
//! `ANCHOR_BACKEND_URL`, on a non-default port.

mod common;

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use common::TestServer;
use serde_json::Value;

fn read_runtime(srv: &TestServer) -> Value {
    let raw = std::fs::read(srv.dir.path().join("runtime.json")).expect("runtime.json written on boot");
    serde_json::from_slice(&raw).expect("runtime.json is JSON")
}

/// One `tools/call` over stdio against `anchorcore mcp --data-dir <dir>`.
fn mcp_call(data_dir: &std::path::Path, tool: &str) -> Value {
    let mut child = Command::new(env!("CARGO_BIN_EXE_anchorcore"))
        .arg("mcp")
        .arg("--data-dir")
        .arg(data_dir)
        .env_remove("ANCHOR_BACKEND_URL")
        .env_remove("ANCHOR_DATA_DIR")
        .env_remove("ANCHOR_DATABASE_URL")
        .env("ANCHOR_SECRETS_NO_KEYRING", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn anchorcore mcp");
    let req = format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{{\"name\":\"{tool}\",\"arguments\":{{}}}}}}\n"
    );
    child.stdin.take().expect("stdin").write_all(req.as_bytes()).expect("write request");
    let out = child.wait_with_output().expect("mcp output");
    let line = String::from_utf8_lossy(&out.stdout).lines().next().unwrap_or_default().to_string();
    serde_json::from_str(&line).unwrap_or_else(|e| panic!("bad MCP reply {line:?}: {e}"))
}

#[tokio::test]
async fn runtime_json_advertises_port_and_mcp_discovers_it() {
    let srv = TestServer::start().await;
    let rt = read_runtime(&srv);
    assert_eq!(rt["port"], srv.port, "{rt}");
    assert_eq!(rt["host"], "127.0.0.1", "{rt}");
    assert_eq!(rt["pid"], srv.pid(), "{rt}");
    assert_eq!(rt["version"], env!("CARGO_PKG_VERSION"), "{rt}");

    // The harness port is random (never 8000): only discovery can reach it.
    // A uniquely named source proves the reply came from this server, not
    // from some app already running on the default port.
    srv.mk_folder_source("r20-4-discovery-probe", &[("a.md", "# A\n")], None).await;
    let dir = srv.dir.path().to_path_buf();
    let reply = tokio::task::spawn_blocking(move || mcp_call(&dir, "list_sources")).await.unwrap();
    assert_eq!(reply["result"]["isError"], false, "{reply}");
    let text = reply["result"]["content"][0]["text"].as_str().unwrap_or_default();
    assert!(text.contains("r20-4-discovery-probe"), "{reply}");
}

#[cfg(unix)]
#[tokio::test]
async fn runtime_json_removed_on_graceful_shutdown() {
    let srv = TestServer::start().await;
    let path = srv.dir.path().join("runtime.json");
    assert!(path.exists());
    let status = Command::new("kill").arg("-TERM").arg(srv.pid().to_string()).status().expect("kill");
    assert!(status.success());
    let deadline = Instant::now() + Duration::from_secs(15);
    while path.exists() && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(!path.exists(), "SIGTERM must remove runtime.json");
}
