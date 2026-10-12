#![allow(dead_code, unused_imports, unused_variables)]
//! AnchorCore MCP mode — stdio transport, 6 read-only tools.
//! R20.8: this is a mode of the single `anchorcore` binary, not a separate
//! sidecar: `anchorcore mcp` (or a copy named `anchorcore-mcp`).
//! Port of `backend/anchorcore_mcp.py` + `backend/app/mcp/server.py` + `tools.py`.
//! Talks to the running app over 127.0.0.1 (single DB owner), like the browser UI.
//!
//! Usage:
//!   anchorcore mcp
//!   claude mcp add anchorcore -- /path/to/anchorcore mcp

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const DEFAULT_URL: &str = "http://127.0.0.1:8000";

static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
fn http_client() -> &'static reqwest::Client {
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("client")
    })
}

static CSRF_TOKEN: OnceLock<tokio::sync::Mutex<Option<String>>> = OnceLock::new();
fn csrf_cache() -> &'static tokio::sync::Mutex<Option<String>> {
    CSRF_TOKEN.get_or_init(|| tokio::sync::Mutex::new(None))
}
async fn get_csrf_token() -> Option<String> {
    {
        let guard = csrf_cache().lock().await;
        if let Some(t) = &*guard { return Some(t.clone()); }
    }
    // fetch fresh
    let base = backend_url();
    let url = format!("{}/csrf", base.trim_end_matches('/'));
    let resp = http_client().get(&url).send().await.ok()?;
    if !resp.status().is_success() { return None; }
    let v: Value = resp.json().await.ok()?;
    let token = v.get("csrf_token")?.as_str()?.to_string();
    let mut guard = csrf_cache().lock().await;
    *guard = Some(token.clone());
    Some(token)
}

fn backend_url() -> String {
    std::env::var("ANCHOR_BACKEND_URL").unwrap_or_else(|_| DEFAULT_URL.to_string())
}
fn backend_token() -> String {
    std::env::var("ANCHOR_MCP_TOKEN").unwrap_or_default()
}

async fn http_call(method: &str, path: &str, body: Option<Value>) -> Result<Value, String> {
    let base = backend_url();
    let token = backend_token();
    let url = format!("{}{}", base.trim_end_matches('/'), path);
    let client = http_client();
    let mut req = match method {
        "GET" => client.get(&url),
        "POST" => client.post(&url),
        _ => return Err(format!("unsupported method {method}")),
    };
    if let Some(b) = body {
        req = req.json(&b);
    }
    if !token.is_empty() {
        req = req.header("Authorization", format!("Bearer {}", token));
    }
    // CSRF for local POST when auth disabled (R7.1)
    if method == "POST" {
        if let Some(csrf) = get_csrf_token().await {
            req = req.header("X-CSRF-Token", csrf);
        }
    }
    // token never appears in URL/path, but redact base if it ever contains token param
    let resp = req.send().await.map_err(|e| format!("backend unreachable ({}). Is the app running?", e))?;
    if resp.status() == 404 {
        return Err("not found".to_string());
    }
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("backend {method} {path} failed ({status}): {}", &text[..text.len().min(500)]));
    }
    resp.json::<Value>().await.map_err(|e| e.to_string())
}

// --- tool implementations (mirrors backend/app/mcp/tools.py) ---

fn audit_mcp(tool: &str, detail: &str) {
    // best-effort audit to system_events (component=mcp) — like Python backend's system_events for MCP
    let data_dir = std::env::var("ANCHOR_DATA_DIR").unwrap_or_else(|_| {
        std::env::var("HOME").map(|h| format!("{}/.anchorcore", h)).unwrap_or_else(|_| "data".to_string())
    });
    let db_path = {
        if let Ok(url) = std::env::var("ANCHOR_DATABASE_URL") {
            if let Some(p) = url.strip_prefix("sqlite:///").or_else(|| url.strip_prefix("sqlite://")) {
                if !p.is_empty() { std::path::PathBuf::from(p) } else { std::path::PathBuf::from(format!("{}/anchorcore.db", data_dir)) }
            } else { std::path::PathBuf::from(format!("{}/anchorcore.db", data_dir)) }
        } else { std::path::PathBuf::from(format!("{}/anchorcore.db", data_dir)) }
    };
    if let Ok(conn) = rusqlite::Connection::open(&db_path) {
        let _ = conn.execute("INSERT INTO system_events (component, level, message, detail) VALUES ('mcp','info',?1,?2)",
            rusqlite::params![format!("mcp tool {}", tool), detail.chars().take(800).collect::<String>()]);
    }
}

async fn tool_ask(question: &str, project_id: Option<i64>, public_only: bool, include_content: bool) -> Result<Value, String> {
    if question.trim().is_empty() {
        return Err("question must be a non-empty string".to_string());
    }
    let mut payload = json!({"question": question.trim()});
    if let Some(pid) = project_id {
        payload["project_id"] = json!(pid);
    }
    if public_only {
        payload["public_only"] = json!(true);
    }
    if include_content {
        payload["include_content"] = json!(true);
    }
    let path = if public_only { "/qa/public" } else { "/qa" };
    let result = http_call("POST", path, Some(payload)).await?;
    audit_mcp("ask", &format!("q={} public_only={} hits={}", question.chars().take(80).collect::<String>(), public_only, result.get("citations").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0)));
    let answer = result.get("answer").and_then(|v| v.as_str()).unwrap_or("").chars().take(4000).collect::<String>();
    let refusal = result.get("refusal").and_then(|v| v.as_str()).map(|s| s.to_string());
    // B58: pass degradation flags through ("unknown" against an older backend).
    let mode = result.get("mode").and_then(|v| v.as_str()).unwrap_or("unknown").to_string();
    let retrieval = result.get("retrieval").and_then(|v| v.as_str()).unwrap_or("unknown").to_string();
    let citations: Vec<Value> = result.get("citations").and_then(|v| v.as_array()).cloned().unwrap_or_default().into_iter().map(|c| {
        // Mirror search: cap citation bytes but say so ("unknown" backends
        // have no content key — empty string, never truncated).
        let content = c.get("content").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let content_truncated = content.chars().count() > 4000;
        json!({
        "entity_id": c.get("entity_id"),
        "item_id": c.get("item_id"),
        "kind": c.get("kind").and_then(|v| v.as_str()).unwrap_or("document"),
        "summary": c.get("summary").and_then(|v| v.as_str()).unwrap_or("").chars().take(500).collect::<String>(),
        "content": content.chars().take(4000).collect::<String>(),
        "content_truncated": content_truncated,
        "source_ref": c.get("source_ref").and_then(|v| v.as_str()).unwrap_or(""),
        "path": c.get("path").and_then(|v| v.as_str()).unwrap_or(""),
        "item_title": c.get("item_title").and_then(|v| v.as_str()).unwrap_or(""),
        "doc_type": c.get("doc_type").and_then(|v| v.as_str()).unwrap_or(""),
        "env": c.get("env").and_then(|v| v.as_str()).unwrap_or("live"),
        "conflicts_with": c.get("conflicts_with").and_then(|v| v.as_array()).cloned().unwrap_or_default(),
        "score": c.get("score"),
        "snippet": c.get("snippet").and_then(|v| v.as_str()).unwrap_or("").chars().take(500).collect::<String>()
        })
    }).collect();
    Ok(json!({"answer": answer, "citations": citations, "refusal": refusal, "mode": mode, "retrieval": retrieval}))
}

async fn tool_search(query: &str, k: usize, project_id: Option<i64>, public_only: bool, env: Option<&str>) -> Result<Value, String> {
    if query.trim().is_empty() {
        return Err("query must be a non-empty string".to_string());
    }
    if k < 1 || k > 50 {
        return Err("k must be between 1 and 50".to_string());
    }
    // The agent composes from these bytes, so opt back into full content
    // (server default is compact citable hits; verbatim drill-down is get_entity).
    let mut payload = json!({"query": query.trim(), "k": k, "fields": "all"});
    if let Some(pid) = project_id {
        payload["project_id"] = json!(pid);
    }
    if public_only {
        payload["public_only"] = json!(true);
    }
    if let Some(e) = env {
        payload["env"] = json!(e);
    }
    let result = http_call("POST", "/qa/search", Some(payload)).await?;
    audit_mcp("search", &format!("q={} k={} public_only={} hits={}", query.chars().take(80).collect::<String>(), k, public_only, result.get("hits").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0)));
    let hits: Vec<Value> = result.get("hits").and_then(|v| v.as_array()).cloned().unwrap_or_default().into_iter().map(|h| {
        // Context budget: cap chunk bytes but say so ("verbatim" must not
        // silently lose the tail — content_truncated lets the agent re-fetch
        // the entity for the rest).
        let content = h.get("content").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let content_truncated = content.chars().count() > 4000;
        json!({
        "chunk_id": h.get("chunk_id"),
        "entity_id": h.get("entity_id"),
        "kind": h.get("kind").and_then(|v| v.as_str()).unwrap_or("document"),
        "summary": h.get("summary").and_then(|v| v.as_str()).unwrap_or("").chars().take(500).collect::<String>(),
        "content": content.chars().take(4000).collect::<String>(),
        "content_truncated": content_truncated,
        "source_ref": h.get("source_ref").and_then(|v| v.as_str()).unwrap_or(""),
        "path": h.get("path").and_then(|v| v.as_str()).unwrap_or(""),
        "doc_type": h.get("doc_type").and_then(|v| v.as_str()).unwrap_or(""),
        "env": h.get("env").and_then(|v| v.as_str()).unwrap_or("live"),
        "conflicts_with": h.get("conflicts_with").and_then(|v| v.as_array()).cloned().unwrap_or_default(),
        "source_id": h.get("source_id"),
        "item_id": h.get("item_id"),
        "item_title": h.get("item_title").and_then(|v| v.as_str()).unwrap_or(""),
        "score": h.get("score")
    })}).collect();
    let refusal = result.get("refusal").and_then(|v| v.as_str()).map(|s| s.to_string());
    let blocked = result.get("blocked").and_then(|v| v.as_u64()).unwrap_or(0);
    let retrieval = result.get("retrieval").and_then(|v| v.as_str()).unwrap_or("unknown").to_string();
    Ok(json!({"query": query, "hits": hits, "blocked": blocked, "refusal": refusal, "retrieval": retrieval}))
}

async fn tool_get_entity(entity_id: i64) -> Result<Value, String> {
    // R35.1: entity reads are ungated direct fetches (the `?gate=provider`
    // honor system is gone with auto-detect). Sensitivity travels with the
    // parent item's manual PII mark + source label, enforced on cloud flows.
    let v = http_call("GET", &format!("/entities/{}", entity_id), None).await.map_err(|e| if e=="not found" { format!("entity {entity_id} not found") } else { e })?;
    Ok(json!({
        "id": v.get("id"),
        "item_id": v.get("item_id"),
        "chunk_id": v.get("chunk_id"),
        "item_is_pii": v.get("item_is_pii"),
        "kind": v.get("kind").and_then(|x| x.as_str()).unwrap_or(""),
        "summary": v.get("summary").and_then(|x| x.as_str()).unwrap_or("").chars().take(1000).collect::<String>(),
        "reasoning": v.get("reasoning").and_then(|x| x.as_str()).unwrap_or("").chars().take(1000).collect::<String>(),
        "confidence": v.get("confidence"),
        "status": v.get("status").and_then(|x| x.as_str()).unwrap_or(""),
        "author": v.get("author").and_then(|x| x.as_str()).unwrap_or(""),
        "owner": v.get("owner").and_then(|x| x.as_str()).unwrap_or(""),
        "source_ref": v.get("source_ref").and_then(|x| x.as_str()).unwrap_or(""),
        "doc_type": v.get("doc_type").and_then(|x| x.as_str()).unwrap_or(""),
        "window_text": v.get("window_text").and_then(|x| x.as_str()).unwrap_or("").chars().take(4000).collect::<String>(),
        "dispute_count": v.get("dispute_count"),
        "created_at": v.get("created_at")
    }))
}

async fn tool_get_source(source_id: i64) -> Result<Value, String> {
    http_call("GET", &format!("/sources/{}", source_id), None).await.map_err(|e| if e=="not found" { format!("source {source_id} not found") } else { e })
}

async fn tool_list_sources() -> Result<Value, String> {
    let v = http_call("GET", "/sources", None).await?;
    let arr = v.as_array().cloned().unwrap_or_default();
    let sources: Vec<Value> = arr.into_iter().map(|s| json!({
        "id": s.get("id"),
        "name": s.get("name").and_then(|x| x.as_str()).unwrap_or(""),
        "connector": s.get("connector").and_then(|x| x.as_str()).unwrap_or(""),
        "enabled": s.get("enabled"),
        "last_synced_at": s.get("last_synced_at"),
        "last_error": s.get("last_error").and_then(|x| x.as_str()).unwrap_or("").chars().take(300).collect::<String>(),
        "error_count": s.get("error_count")
    })).collect();
    Ok(json!({"sources": sources}))
}

async fn tool_memory_status() -> Result<Value, String> {
    http_call("GET", "/system/status", None).await
}

// --- MCP JSON-RPC handling ---

#[derive(Deserialize)]
struct Request {
    jsonrpc: String,
    id: Option<Value>,
    method: String,
    params: Option<Value>,
}

#[derive(Serialize)]
struct Response {
    jsonrpc: String,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<Value>,
}

fn tool_defs() -> Value {
    json!([
        {
            "name": "ask",
            "description": "Policy-gated composed answer from AnchorCore memory, with citations. Use when the material may be sensitive/PII or you want the server to enforce PII/provider policy before you see bytes. Otherwise prefer search so your own model composes from verbatim cited chunks.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "question": {"type": "string", "description": "Question to answer"},
                    "project_id": {"type": "integer", "description": "Optional project id to scope retrieval"},
                    "public_only": {"type": "boolean", "description": "If true, only public sources are searched — share-safe"},
                    "include_content": {"type": "boolean", "description": "If true, citations carry full chunk bytes (capped at 4000 chars; content_truncated says so)"}
                },
                "required": ["question"]
            }
        },
        {
            "name": "search",
            "description": "Default extraction: gated, cited chunks (with provenance, item title/path and scores) for a query. Prefer this — your model composes the answer from the returned bytes. Long chunks are capped at 4000 chars and set content_truncated=true. A \"refusal\" field carries \"gated_provider\" when sensitive content was withheld; \"blocked\" counts withheld hits.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {"type": "string"},
                    "k": {"type": "integer", "default": 8, "minimum": 1, "maximum": 50},
                    "project_id": {"type": "integer"},
                    "public_only": {"type": "boolean", "description": "If true, only public sources are searched"},
                    "env": {"type": "string", "description": "Optional env class filter: 'live' or 'test' (path- or header-derived; test fixtures rank below live hits by default, not hidden)"}
                },
                "required": ["query"]
            }
        },
        {
            "name": "get_entity",
            "description": "Fetch one entity with provenance (summary, confidence, status, window_text, source_ref, parent item's manual PII mark).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "entity_id": {"type": "integer"}
                },
                "required": ["entity_id"]
            }
        },
        {
            "name": "get_source",
            "description": "Fetch one source's details (connector, label, sync status, recent errors). Config and secrets are not returned.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "source_id": {"type": "integer"}
                },
                "required": ["source_id"]
            }
        },
        {
            "name": "list_sources",
            "description": "List all connected sources.",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        },
        {
            "name": "memory_status",
            "description": "Memory health: version, model availability, pending embeddings.",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        }
    ])
}

async fn handle_request(req: Request) -> Option<Response> {
    let id = req.id.clone().unwrap_or(Value::Null);
    let is_notification = req.id.is_none();
    match req.method.as_str() {
        "initialize" => {
            let result = json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "anchorcore", "version": env!("CARGO_PKG_VERSION")}
            });
            Some(Response { jsonrpc: "2.0".to_string(), id, result: Some(result), error: None })
        }
        "notifications/initialized" | "initialized" => None,
        "ping" => Some(Response { jsonrpc: "2.0".to_string(), id, result: Some(json!({})), error: None }),
        "tools/list" => {
            let result = json!({"tools": tool_defs()});
            Some(Response { jsonrpc: "2.0".to_string(), id, result: Some(result), error: None })
        }
        "tools/call" => {
            let params = req.params.unwrap_or(json!({}));
            let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            let res: Result<Value, String> = match name {
                "ask" => {
                    let q = args.get("question").and_then(|v| v.as_str()).unwrap_or("");
                    let pid = args.get("project_id").and_then(|v| v.as_i64());
                    let pub_only = args.get("public_only").and_then(|v| v.as_bool()).unwrap_or(false);
                    let want_content = args.get("include_content").and_then(|v| v.as_bool()).unwrap_or(false);
                    tool_ask(q, pid, pub_only, want_content).await
                }
                "search" => {
                    let q = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
                    let k = args.get("k").and_then(|v| v.as_u64()).unwrap_or(8) as usize;
                    let pid = args.get("project_id").and_then(|v| v.as_i64());
                    let pub_only = args.get("public_only").and_then(|v| v.as_bool()).unwrap_or(false);
                    let env = args.get("env").and_then(|v| v.as_str());
                    tool_search(q, k, pid, pub_only, env).await
                }
                "get_entity" => {
                    let eid = args.get("entity_id").and_then(|v| v.as_i64()).unwrap_or(0);
                    tool_get_entity(eid).await
                }
                "get_source" => {
                    let sid = args.get("source_id").and_then(|v| v.as_i64()).unwrap_or(0);
                    tool_get_source(sid).await
                }
                "list_sources" => tool_list_sources().await,
                "memory_status" => tool_memory_status().await,
                _ => Err(format!("unknown tool {name}")),
            };
            match res {
                Ok(val) => {
                    let result = json!({"content": [{"type": "text", "text": serde_json::to_string_pretty(&val).unwrap_or_default()}], "isError": false});
                    Some(Response { jsonrpc: "2.0".to_string(), id, result: Some(result), error: None })
                }
                Err(e) => {
                    let result = json!({"content": [{"type": "text", "text": e}], "isError": true});
                    Some(Response { jsonrpc: "2.0".to_string(), id, result: Some(result), error: None })
                }
            }
        }
        _ => {
            if is_notification {
                None
            } else {
                Some(Response { jsonrpc: "2.0".to_string(), id, result: None, error: Some(json!({"code": -32601, "message": format!("Method not found: {}", req.method)})) })
            }
        }
    }
}

pub async fn run() {
    let stdin = tokio::io::stdin();
    let mut reader = tokio::io::BufReader::new(stdin);
    let mut stdout = tokio::io::stdout();
    let mut line = String::new();
    loop {
        line.clear();
        let n = {
            use tokio::io::AsyncBufReadExt;
            match reader.read_line(&mut line).await {
                Ok(0) => break,
                Ok(n) => n,
                Err(_) => break,
            }
        };
        if n == 0 { break; }
        let trimmed = line.trim();
        if trimmed.is_empty() { continue; }
        let req: Result<Request, _> = serde_json::from_str(trimmed);
        let req = match req {
            Ok(r) => r,
            Err(e) => {
                let err = json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":format!("Parse error: {e}")}});
                use tokio::io::AsyncWriteExt;
                let _ = stdout.write_all(format!("{}\n", err).as_bytes()).await;
                let _ = stdout.flush().await;
                continue;
            }
        };
        if let Some(resp) = handle_request(req).await {
            let out = serde_json::to_string(&resp).unwrap();
            use tokio::io::AsyncWriteExt;
            let _ = stdout.write_all(format!("{}\n", out).as_bytes()).await;
            let _ = stdout.flush().await;
        }
    }
}
