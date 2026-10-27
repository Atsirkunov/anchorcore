//! API contract: agent read APIs (R36.15, black-box) — line windows on
//! `GET /items/:id?lines=`, section text on `GET /nodes/:id?include=text`,
//! `path_prefix` rejection on `GET /items` + `POST /qa/search`, the
//! `public_only` scope and the label gate on byte reads, and the matching
//! MCP tools (`list_items`, `get_item`, `get_outline`) over stdio.

mod common;

use std::io::Write;
use std::process::{Command, Stdio};

use common::TestServer;
use reqwest::StatusCode;
use serde_json::{Value, json};

const RUNBOOK: &str = "# Runbook\n\nIntro line about the billing cutover.\n\n## Rollback\n\nStep one: drain the settlement queue.\nStep two: revert the routing flag.\n\n## Contacts\n\nPager rotation owner is the payments team.\n";
const ROUTING: &str = "tenants:\n  - id: acme\n    settle_code: AC1\n    fee: 3\n  - id: globex\n    settle_code: GL2\n    fee: 5\nretries: 3\ntimeout_ms: 2500\n";

/// Folder source with the runbook + routing config, synced. Returns
/// (source id, runbook item id, routing item id).
async fn synced(srv: &TestServer, name: &str, label: Option<&str>) -> (i64, i64, i64) {
    let source = srv.mk_folder_source(name, &[("runbook.md", RUNBOOK), ("routing.yml", ROUTING)], label).await;
    let sid = source["id"].as_i64().expect("source id");
    let job = srv.sync_and_wait(sid, "sync").await;
    assert_eq!(job["status"], "done", "{job}");
    let listing = srv.get_json(&format!("/items?source_id={sid}")).await;
    let id_of = |path: &str| {
        listing["items"].as_array().unwrap().iter().find(|i| i["path"] == path).and_then(|i| i["id"].as_i64()).unwrap_or_else(|| panic!("{path} not listed: {listing}"))
    };
    (sid, id_of("runbook.md"), id_of("routing.yml"))
}

async fn status_and_body(srv: &TestServer, path: &str) -> (StatusCode, Value) {
    let resp = srv.get(path).await;
    let status = resp.status();
    (status, resp.json().await.unwrap_or(Value::Null))
}

fn section_by_title(outline: &Value, title: &str) -> Value {
    outline["sections"].as_array().unwrap().iter().find(|s| s["title"] == title).cloned().unwrap_or_else(|| panic!("no section {title}: {outline}"))
}

#[tokio::test]
async fn line_windows_serve_ranges_and_reject_invalid_ones() {
    let srv = TestServer::start().await;
    let (_sid, runbook, routing) = synced(&srv, "read-lines", None).await;

    let v = srv.get_json(&format!("/items/{runbook}?lines=7-8")).await;
    assert_eq!(v["content"], "Step one: drain the settlement queue.\nStep two: revert the routing flag.", "{v}");
    assert_eq!(v["text"], v["content"], "text stays the alias");
    assert_eq!(v["lines"], json!({"start": 7, "end": 8, "total": 12}));
    assert_eq!(v["path"], "runbook.md", "metadata travels with the window");

    let v = srv.get_json(&format!("/items/{runbook}?lines=12-99")).await;
    assert_eq!(v["lines"]["end"], 12, "an end past EOF clamps: {v}");
    let v = srv.get_json(&format!("/items/{runbook}?lines=12-")).await;
    assert_eq!(v["content"], "Pager rotation owner is the payments team.", "{v}");
    let v = srv.get_json(&format!("/items/{runbook}?lines=1")).await;
    assert_eq!(v["content"], "# Runbook");

    for bad in ["8-7", "0-2", "abc", "", "1..3"] {
        let (status, body) = status_and_body(&srv, &format!("/items/{runbook}?lines={bad}")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}: {body}");
        assert!(body["detail"].as_str().unwrap_or("").contains("\"a-b\""), "{bad}: {body}");
    }
    let (status, body) = status_and_body(&srv, &format!("/items/{runbook}?lines=40-50")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["detail"].as_str().unwrap().contains("12 line(s)"), "{body}");
    let (status, body) = status_and_body(&srv, &format!("/items/{}?lines=1-2", routing + 999)).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["detail"], "Item not found");

    // R36.2 config sections carry line spans: the outline's span is a
    // window over exactly those source lines.
    let outline = srv.get_json(&format!("/items/{routing}/outline")).await;
    let sections = outline["sections"].as_array().expect("sections");
    let sec = sections.iter().find(|s| s["line_start"].as_i64().unwrap_or(0) > 0).unwrap_or_else(|| panic!("no config span: {outline}"));
    let (a, b) = (sec["line_start"].as_i64().unwrap() as usize, sec["line_end"].as_i64().unwrap() as usize);
    let v = srv.get_json(&format!("/items/{routing}?lines={a}-{b}")).await;
    let want: Vec<&str> = ROUTING.lines().skip(a - 1).take(b - a + 1).collect();
    assert_eq!(v["content"], want.join("\n"), "section {sec}");
}

#[tokio::test]
async fn node_text_returns_ordered_subtree_leaves_with_a_truncation_flag() {
    let srv = TestServer::start().await;
    let (_sid, runbook, _routing) = synced(&srv, "read-nodes", None).await;
    let outline = srv.get_json(&format!("/items/{runbook}/outline")).await;
    let rollback = section_by_title(&outline, "Rollback");
    let nid = rollback["id"].as_i64().unwrap();

    let bare = srv.get_json(&format!("/nodes/{nid}")).await;
    assert!(bare.get("text").is_none(), "text is opt-in: {bare}");
    let v = srv.get_json(&format!("/nodes/{nid}?include=text,parent")).await;
    let text = v["text"].as_str().expect("text");
    assert!(text.contains("drain the settlement queue") && text.contains("revert the routing flag"), "{v}");
    assert!(text.find("drain").unwrap() < text.find("revert").unwrap(), "document order");
    assert!(!text.contains("Pager rotation"), "a sibling section's bytes never leak in: {v}");
    assert_eq!(v["text_truncated"], false);
    assert_eq!(v["text_chars"].as_u64(), Some(text.chars().count() as u64));
    assert!(v.get("parent").is_some(), "links combine with text: {v}");

    let cut = srv.get_json(&format!("/nodes/{nid}?include=text&max_chars=5")).await;
    assert_eq!(cut["text"].as_str().map(|t| t.chars().count()), Some(5));
    assert_eq!(cut["text_truncated"], true);
    assert_eq!(cut["text_chars"], v["text_chars"]);

    // The document root spans every child section.
    if let Some(root) = outline["sections"].as_array().unwrap().iter().find(|s| s["title"] == "Runbook") {
        let all = srv.get_json(&format!("/nodes/{}?include=text", root["id"])).await;
        let t = all["text"].as_str().unwrap();
        assert!(t.contains("drain the settlement queue") && t.contains("Pager rotation"), "{all}");
    }
    let (status, body) = status_and_body(&srv, &format!("/nodes/{}?include=text", nid + 999)).await;
    assert_eq!((status, body["detail"].as_str()), (StatusCode::NOT_FOUND, Some("Node not found")));
}

#[tokio::test]
async fn malformed_path_prefix_is_a_400_naming_the_expected_form() {
    let srv = TestServer::start().await;
    synced(&srv, "read-prefix", None).await;
    for bad in ["docs/*.md", "../etc", "./runbook", "~/notes", "file:///tmp/x", "C:docs"] {
        let enc = bad.replace('/', "%2F").replace('*', "%2A").replace('~', "%7E").replace(':', "%3A");
        let (status, body) = status_and_body(&srv, &format!("/items?path_prefix={enc}")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}: {body}");
        let detail = body["detail"].as_str().unwrap_or("");
        assert!(detail.contains("Expected a relative item path"), "{bad}: {body}");

        let resp = srv.post("/qa/search", &json!({"query": "rollback", "path_prefix": bad})).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "search {bad}");
        let body: Value = resp.json().await.unwrap();
        assert!(body["detail"].as_str().unwrap_or("").contains("malformed"), "{body}");
        let resp = srv.post("/qa/search", &json!({"query": "rollback", "path_prefix": bad, "mode": "exact"})).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "exact search {bad}");
    }
    // Well-formed prefixes keep working, matches or not.
    let v = srv.get_json("/items?path_prefix=runbook").await;
    assert_eq!(v["total"], 1, "{v}");
    let v = srv.get_json("/items?path_prefix=nothing-here/").await;
    assert_eq!(v["total"], 0, "well-formed but empty stays an empty list");
    let resp = srv.post("/qa/search", &json!({"query": "rollback", "path_prefix": "runbook"})).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let resp = srv.post("/qa/search", &json!({"query": "rollback", "path_prefix": ""})).await;
    assert_eq!(resp.status(), StatusCode::OK, "empty prefix = unfiltered");
}

#[tokio::test]
async fn byte_reads_follow_the_label_gate_and_public_scope() {
    // Default harness provider is an untrusted cloud URL.
    let srv = TestServer::start().await;
    let (_s, secret, _) = synced(&srv, "read-sensitive", Some("sensitive")).await;
    let (_i, internal, _) = synced(&srv, "read-internal", None).await;
    let (public_sid, public, _) = synced(&srv, "read-public", Some("public")).await;

    let (status, body) = status_and_body(&srv, &format!("/items/{secret}?lines=1-3")).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["refusal"], "gated_provider");
    assert!(body["detail"].is_string());
    let outline = srv.get_json(&format!("/items/{secret}/outline")).await;
    let nid = section_by_title(&outline, "Rollback")["id"].as_i64().unwrap();
    let (status, body) = status_and_body(&srv, &format!("/nodes/{nid}?include=text")).await;
    assert_eq!((status, body["refusal"].as_str()), (StatusCode::FORBIDDEN, Some("gated_provider")), "{body}");
    let meta = srv.get_json(&format!("/nodes/{nid}?include=parent")).await;
    assert!(meta.get("text").is_none(), "metadata stays readable: {meta}");
    let events: i64 = srv.db().query_row("SELECT COUNT(*) FROM system_events WHERE message LIKE 'answer gate:%'", [], |r| r.get(0)).unwrap();
    assert!(events >= 2, "each refusal is audited ({events})");
    // Internal + public bytes flow under the same provider.
    srv.get_json(&format!("/items/{internal}?lines=1-3")).await;

    // public_only: anything outside a public source reads as not found.
    for path in [format!("/items/{internal}?public_only=true"), format!("/items/{internal}/outline?public_only=true"), format!("/items/{internal}?lines=1&public_only=true")] {
        let (status, body) = status_and_body(&srv, &path).await;
        assert_eq!((status, body["detail"].as_str()), (StatusCode::NOT_FOUND, Some("Item not found")), "{path}");
    }
    let (status, _) = status_and_body(&srv, &format!("/nodes/{nid}?public_only=true")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    srv.get_json(&format!("/items/{public}?lines=1&public_only=true")).await;
    let pub_outline = srv.get_json(&format!("/items/{public}/outline?public_only=true")).await;
    let pub_node = section_by_title(&pub_outline, "Rollback")["id"].as_i64().unwrap();
    let v = srv.get_json(&format!("/nodes/{pub_node}?include=text&public_only=true")).await;
    assert!(v["text"].as_str().unwrap().contains("drain"), "{v}");
    let listed = srv.get_json("/items?public_only=true").await;
    assert_eq!(listed["total"], 2, "{listed}");
    assert!(listed["items"].as_array().unwrap().iter().all(|i| i["source_id"] == public_sid), "{listed}");
}

#[tokio::test]
async fn trusted_provider_reads_sensitive_windows() {
    let srv = TestServer::start_with_env(&[("ANCHOR_CLOUD_TRUST", "1")]).await;
    let (_s, secret, _) = synced(&srv, "read-trusted", Some("sensitive")).await;
    let v = srv.get_json(&format!("/items/{secret}?lines=1")).await;
    assert_eq!(v["content"], "# Runbook");
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
async fn mcp_read_tools_reach_the_same_rest_contract() {
    let srv = TestServer::start().await;
    let (_sid, runbook, _routing) = synced(&srv, "read-mcp", None).await;
    let (_g, secret, _) = synced(&srv, "read-mcp-sensitive", Some("sensitive")).await;
    let base = srv.url("");
    let call = |tool: &'static str, args: Value| {
        let base = base.clone();
        tokio::task::spawn_blocking(move || mcp_call(&base, tool, args))
    };

    let reply = call("list_items", json!({"path_prefix": "runbook"})).await.unwrap();
    assert_eq!(reply["result"]["isError"], false, "{reply}");
    let listed: Value = serde_json::from_str(&tool_text(&reply)).unwrap();
    assert!(listed["items"].as_array().unwrap().iter().any(|i| i["id"] == runbook), "{listed}");

    let reply = call("list_items", json!({"path_prefix": "docs/*.md"})).await.unwrap();
    assert_eq!(reply["result"]["isError"], true, "{reply}");
    assert!(tool_text(&reply).contains("Expected a relative item path"), "{reply}");

    let reply = call("get_item", json!({"item_id": runbook, "lines": "7-8"})).await.unwrap();
    assert_eq!(reply["result"]["isError"], false, "{reply}");
    let item: Value = serde_json::from_str(&tool_text(&reply)).unwrap();
    assert_eq!(item["content"], "Step one: drain the settlement queue.\nStep two: revert the routing flag.");
    assert_eq!(item["lines"]["total"], 12);
    assert!(item.get("text").is_none(), "MCP sends the bytes once");

    let reply = call("get_item", json!({"item_id": runbook, "lines": "9-2"})).await.unwrap();
    assert_eq!(reply["result"]["isError"], true);
    assert!(tool_text(&reply).contains("400"), "{reply}");
    let reply = call("get_item", json!({"item_id": secret, "lines": "1"})).await.unwrap();
    assert!(tool_text(&reply).contains("gated_provider"), "{reply}");
    let reply = call("get_item", json!({"item_id": runbook, "public_only": true})).await.unwrap();
    assert_eq!(tool_text(&reply), format!("item {runbook} not found"));

    let reply = call("get_outline", json!({"item_id": runbook})).await.unwrap();
    let outline: Value = serde_json::from_str(&tool_text(&reply)).unwrap();
    let nid = section_by_title(&outline, "Rollback")["id"].as_i64().unwrap();
    let reply = call("get_outline", json!({"node_id": nid, "include": "text,parent"})).await.unwrap();
    assert_eq!(reply["result"]["isError"], false, "{reply}");
    let node: Value = serde_json::from_str(&tool_text(&reply)).unwrap();
    assert!(node["text"].as_str().unwrap().contains("drain the settlement queue"), "{node}");
    assert_eq!(node["node"]["id"], nid);
}
