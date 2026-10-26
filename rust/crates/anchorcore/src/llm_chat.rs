//! One chat call for the local-mode answer path (R30.6): `generate_answer`,
//! the follow-up rewrite and query expansion all go through `chat`.
//!
//! When the answer provider is a local Ollama, the call uses the native
//! `/api/chat` endpoint with `options.num_ctx` pinned: Ollama ≥ 0.15.5 sizes
//! the context from free memory (32k tokens on a 32 GB Mac → a 3B model at
//! ~12 GB) and its OpenAI-compatible endpoint ignores context options
//! (ollama/ollama#18229). Every other provider (cloud, BYO OpenAI-compatible,
//! other local servers) keeps `{base}/chat/completions` byte-for-byte.
//! Agent mode never reaches this module (zero chat calls).

use std::time::Duration;

use serde_json::{json, Value};

/// Context pinned on local-Ollama calls: the 6000-token generation budget
/// (`answer.rs` R43.7) plus prompt and answer headroom.
pub const DEFAULT_NUM_CTX: u64 = 8192;
const OLLAMA_PORT: u16 = 11434;

/// One chat request: messages plus the sampling knobs the callers use.
pub struct ChatSpec<'a> {
    pub base: &'a str,
    pub api_key: &'a str,
    pub model: &'a str,
    pub messages: Value,
    pub temperature: Option<f64>,
    pub max_tokens: Option<u64>,
    pub timeout: Duration,
    /// Ollama's base URL setting (`ollama_base_url`), used to recognise it
    /// behind a non-default port.
    pub ollama_base: Option<&'a str>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ChatError {
    /// Transport error, timeout or non-success status.
    Unreachable,
    /// The model answered without any message content.
    Empty,
}

/// Built request: where to POST, what to send, and how to read the reply.
#[derive(Debug)]
pub struct ChatRequest {
    pub url: String,
    pub payload: Value,
    pub native: bool,
}

fn url_root(base: &str) -> &str {
    let trimmed = base.trim_end_matches('/');
    trimmed.strip_suffix("/v1").unwrap_or(trimmed)
}

/// True when `base` is a loopback Ollama: the default port, or the same root
/// as the configured `ollama_base_url`.
pub fn is_local_ollama(base: &str, ollama_base: Option<&str>) -> bool {
    if !crate::common::provider_is_local(base) {
        return false;
    }
    let default_port = reqwest::Url::parse(base)
        .ok()
        .and_then(|u| u.port_or_known_default())
        .is_some_and(|p| p == OLLAMA_PORT);
    default_port || ollama_base.is_some_and(|o| url_root(o) == url_root(base))
}

/// `ANCHOR_OLLAMA_NUM_CTX` overrides the pinned context (positive integer).
pub fn num_ctx() -> u64 {
    std::env::var("ANCHOR_OLLAMA_NUM_CTX")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(DEFAULT_NUM_CTX)
}

/// The OpenAI-compatible request every non-Ollama provider gets (unchanged).
pub fn openai_request(spec: &ChatSpec<'_>) -> ChatRequest {
    let mut payload = json!({"model": spec.model, "messages": spec.messages});
    if let Some(t) = spec.temperature {
        payload["temperature"] = json!(t);
    }
    if let Some(n) = spec.max_tokens {
        payload["max_tokens"] = json!(n);
    }
    ChatRequest { url: format!("{}/chat/completions", spec.base.trim_end_matches('/')), payload, native: false }
}

/// Native Ollama `/api/chat`, non-streaming, with `num_ctx` pinned.
pub fn ollama_request(spec: &ChatSpec<'_>, ctx: u64) -> ChatRequest {
    let mut options = json!({"num_ctx": ctx});
    if let Some(t) = spec.temperature {
        options["temperature"] = json!(t);
    }
    if let Some(n) = spec.max_tokens {
        options["num_predict"] = json!(n);
    }
    let payload = json!({"model": spec.model, "messages": spec.messages, "stream": false, "options": options});
    ChatRequest { url: format!("{}/api/chat", url_root(spec.base)), payload, native: true }
}

/// Pick the request shape for `spec` (native only for a local Ollama).
pub fn build_request(spec: &ChatSpec<'_>) -> ChatRequest {
    if is_local_ollama(spec.base, spec.ollama_base) {
        ollama_request(spec, num_ctx())
    } else {
        openai_request(spec)
    }
}

/// Message text from either reply shape.
pub fn reply_content(body: &Value, native: bool) -> Option<&str> {
    let pointer = if native { "/message/content" } else { "/choices/0/message/content" };
    body.pointer(pointer).and_then(|v| v.as_str())
}

enum Sent {
    Reply(Value),
    /// The endpoint does not exist (404/405): not an Ollama after all.
    Missing,
}

async fn post(client: &reqwest::Client, req: &ChatRequest, api_key: &str) -> reqwest::Result<reqwest::Response> {
    let mut builder = client.post(&req.url).json(&req.payload);
    if !api_key.is_empty() {
        builder = builder.bearer_auth(api_key);
    }
    builder.send().await
}

fn endpoint_missing(req: &ChatRequest, status: reqwest::StatusCode) -> bool {
    req.native && matches!(status.as_u16(), 404 | 405)
}

async fn send(client: &reqwest::Client, req: &ChatRequest, api_key: &str) -> Result<Sent, ChatError> {
    let resp = post(client, req, api_key).await.map_err(|_| ChatError::Unreachable)?;
    let status = resp.status();
    if endpoint_missing(req, status) {
        return Ok(Sent::Missing);
    }
    if !status.is_success() {
        return Err(ChatError::Unreachable);
    }
    Ok(Sent::Reply(resp.json::<Value>().await.map_err(|_| ChatError::Empty)?))
}

/// Run one chat call. A native request whose endpoint is missing (a
/// non-Ollama server on 11434) retries once as OpenAI-compatible.
pub async fn chat(spec: &ChatSpec<'_>) -> Result<String, ChatError> {
    let client = reqwest::Client::builder().timeout(spec.timeout).build().map_err(|_| ChatError::Unreachable)?;
    let mut req = build_request(spec);
    let mut sent = send(&client, &req, spec.api_key).await?;
    if matches!(sent, Sent::Missing) {
        req = openai_request(spec);
        sent = send(&client, &req, spec.api_key).await?;
    }
    let Sent::Reply(body) = sent else { return Err(ChatError::Unreachable) };
    reply_content(&body, req.native).map(str::to_string).ok_or(ChatError::Empty)
}

/// Settings "test answer model" ping: same request shape (and the same
/// pinned context, so the probe does not load the model at the auto size),
/// reporting the raw failure for the UI.
pub async fn ping(spec: &ChatSpec<'_>, client: &reqwest::Client) -> Result<(), String> {
    let mut req = build_request(spec);
    let mut resp = post(client, &req, spec.api_key).await.map_err(|e| format!("request failed: {e}"))?;
    if endpoint_missing(&req, resp.status()) {
        req = openai_request(spec);
        resp = post(client, &req, spec.api_key).await.map_err(|e| format!("request failed: {e}"))?;
    }
    if resp.status().is_success() {
        Ok(())
    } else {
        Err(format!("request failed: status {}", resp.status()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec<'a>(base: &'a str, ollama_base: Option<&'a str>) -> ChatSpec<'a> {
        ChatSpec {
            base,
            api_key: "",
            model: "llama3.2:3b",
            messages: json!([{"role": "user", "content": "hi"}]),
            temperature: Some(0.2),
            max_tokens: Some(120),
            timeout: Duration::from_secs(1),
            ollama_base,
        }
    }

    #[test]
    fn local_ollama_gets_native_chat_with_num_ctx() {
        let req = build_request(&spec("http://localhost:11434/v1", None));
        assert!(req.native);
        assert_eq!(req.url, "http://localhost:11434/api/chat");
        assert_eq!(req.payload["stream"], false);
        assert_eq!(req.payload["options"]["num_ctx"], DEFAULT_NUM_CTX);
        assert_eq!(req.payload["options"]["num_predict"], 120);
        assert_eq!(req.payload["options"]["temperature"], 0.2);
        assert!(req.payload.get("max_tokens").is_none());
    }

    #[test]
    fn ollama_on_custom_port_is_recognised_by_its_base_setting() {
        let req = build_request(&spec("http://127.0.0.1:9999/v1/", Some("http://127.0.0.1:9999")));
        assert!(req.native);
        assert_eq!(req.url, "http://127.0.0.1:9999/api/chat");
    }

    #[test]
    fn cloud_and_other_local_servers_keep_openai_shape() {
        for base in ["https://api.openai.com/v1", "http://localhost:1234/v1", "http://localhost.evil.example:11434/v1"] {
            let req = build_request(&spec(base, Some("http://localhost:11434")));
            assert!(!req.native, "{base}");
            assert_eq!(req.url, format!("{}/chat/completions", base.trim_end_matches('/')));
            assert_eq!(req.payload["max_tokens"], 120);
            assert!(req.payload.get("options").is_none(), "{base}: no Ollama options on OpenAI calls");
            assert!(req.payload.get("stream").is_none());
        }
    }

    #[test]
    fn max_tokens_omitted_when_unset() {
        let mut s = spec("https://api.openai.com/v1", None);
        s.max_tokens = None;
        s.temperature = None;
        assert!(openai_request(&s).payload.get("max_tokens").is_none());
        assert!(openai_request(&s).payload.get("temperature").is_none());
        assert!(ollama_request(&s, 4096).payload["options"].get("temperature").is_none());
        assert!(ollama_request(&s, 4096).payload["options"].get("num_predict").is_none());
        assert_eq!(ollama_request(&s, 4096).payload["options"]["num_ctx"], 4096);
    }

    #[test]
    fn reply_content_reads_both_shapes() {
        let native = json!({"message": {"role": "assistant", "content": "a"}});
        let openai = json!({"choices": [{"message": {"content": "b"}}]});
        assert_eq!(reply_content(&native, true), Some("a"));
        assert_eq!(reply_content(&openai, false), Some("b"));
        assert_eq!(reply_content(&openai, true), None);
    }

    /// One-shot HTTP server: answers each accepted connection with the next
    /// canned (status, body) and records request lines.
    async fn mock(replies: Vec<(u16, &'static str)>) -> (u16, tokio::task::JoinHandle<Vec<String>>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = tokio::spawn(async move {
            let mut seen = Vec::new();
            for (status, body) in replies {
                let Ok((mut sock, _)) = listener.accept().await else { break };
                let mut buf = vec![0u8; 16384];
                let n = sock.read(&mut buf).await.unwrap_or(0);
                let text = String::from_utf8_lossy(&buf[..n]).to_string();
                seen.push(text.lines().next().unwrap_or("").to_string() + " " + text.split("\r\n\r\n").nth(1).unwrap_or(""));
                let head = format!("HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n", body.len());
                let _ = sock.write_all(head.as_bytes()).await;
                let _ = sock.write_all(body.as_bytes()).await;
            }
            seen
        });
        (port, handle)
    }

    #[tokio::test]
    async fn native_call_parses_message_content() {
        let (port, handle) = mock(vec![(200, r#"{"message":{"role":"assistant","content":"native answer"}}"#)]).await;
        let base = format!("http://127.0.0.1:{port}/v1");
        let ollama = format!("http://127.0.0.1:{port}");
        assert_eq!(chat(&spec(&base, Some(&ollama))).await, Ok("native answer".to_string()));
        let seen = handle.await.unwrap();
        assert!(seen[0].starts_with("POST /api/chat "), "{seen:?}");
        assert!(seen[0].contains("\"num_ctx\":8192"), "{seen:?}");
    }

    #[tokio::test]
    async fn missing_native_endpoint_falls_back_to_openai() {
        let (port, handle) = mock(vec![(404, "{}"), (200, r#"{"choices":[{"message":{"content":"compat"}}]}"#)]).await;
        let base = format!("http://127.0.0.1:{port}/v1");
        let ollama = format!("http://127.0.0.1:{port}");
        assert_eq!(chat(&spec(&base, Some(&ollama))).await, Ok("compat".to_string()));
        let seen = handle.await.unwrap();
        assert!(seen[1].starts_with("POST /v1/chat/completions "), "{seen:?}");
    }
}
