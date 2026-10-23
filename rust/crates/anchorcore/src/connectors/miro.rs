//! Miro connector (R46.5) — REST v2, one item per board, rendered through the
//! R46.1 diagram normal form so boards chunk like any markdown document.
//!
//! Config: `token` (access token — stripped into the SecretStore at create
//! and filled back by `pipeline::resolve_config`, never in the stored
//! config), `boards` (board ids: comma list or JSON array) and/or `team_id`
//! (every board of the team), `base_url` (default `https://api.miro.com`).
//!
//! - Items (`sticky_note`, `shape`, `text`, `card`, `app_card`) become nodes,
//!   `frame` items become frames (membership via `parent.id`), connectors
//!   become `A → B: caption` edges. HTML in content is stripped; images,
//!   embeds and documents carry no text and are skipped (R46.13 owns images).
//! - Incremental: the board's `modifiedAt` is compared with the stamp the
//!   pipeline recorded for a fully processed item; an unchanged board is
//!   reported as `Unchanged` and its items are never fetched.
//! - Citations: `source_ref` is the board URL; frames are reachable with
//!   `…/?moveToWidget=<frame id>`.
//! - 429: bounded retries honoring `Retry-After`, else `X-RateLimit-Reset`
//!   (epoch seconds), else exponential backoff — each wait capped.
//! - An explicitly listed board that answers 404 is skipped (it was
//!   deleted), so the pipeline purges its item; any other error fails the
//!   sync (no purge on a partial view).

use std::collections::HashMap;

use reqwest::Client;
use serde_json::Value;

use super::{ConnectorError, IngestionDoc};
use crate::diagram::{Diagram, Edge, Frame, Node};

const DEFAULT_BASE: &str = "https://api.miro.com";
const PAGE_LIMIT: usize = 50;
/// Hard stop for runaway pagination (50 × 2000 = 100k items per board).
const MAX_PAGES: usize = 2000;
const MAX_RETRIES: u32 = 4;
const MAX_WAIT_SECS: u64 = 60;

/// One board's outcome: fresh text, or "unchanged since the stamp".
#[derive(Debug, Clone)]
pub enum BoardFetch {
    Fresh { doc: IngestionDoc, modified_at: String },
    Unchanged { board_id: String, modified_at: String },
}

pub fn external_id(board_id: &str) -> String {
    format!("miro:{board_id}")
}

pub fn board_url(board_id: &str) -> String {
    format!("https://miro.com/app/board/{board_id}/")
}

pub struct MiroConnector {
    token: String,
    base_url: String,
    boards: Vec<String>,
    team_id: String,
    client: Client,
}

fn config_list(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::Array(a)) => a.iter().filter_map(|x| x.as_str()).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
        Some(Value::String(s)) => s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect(),
        _ => Vec::new(),
    }
}

/// Path-segment encoding: Miro board ids carry `=` and other non-path-safe bytes.
fn encode_segment(s: &str) -> String {
    s.bytes()
        .map(|b| if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect()
}

fn str_at<'a>(v: &'a Value, ptr: &str) -> &'a str {
    v.pointer(ptr).and_then(|x| x.as_str()).unwrap_or("")
}

fn plain(html: &str) -> String {
    if html.contains('<') || html.contains('&') {
        crate::extract::strip_html(html)
    } else {
        html.to_string()
    }
}

/// Seconds to wait before retrying a 429.
fn retry_wait(headers: &reqwest::header::HeaderMap, attempt: u32) -> u64 {
    let header = |name: &str| headers.get(name).and_then(|v| v.to_str().ok()).and_then(|s| s.trim().parse::<u64>().ok());
    let wait = header("retry-after").or_else(|| {
        let reset = header("x-ratelimit-reset")?;
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
        Some(reset.saturating_sub(now))
    });
    wait.unwrap_or(1u64 << attempt.min(5)).min(MAX_WAIT_SECS)
}

impl MiroConnector {
    pub fn new(config: &Value) -> Result<Self, ConnectorError> {
        let token = config
            .get("token")
            .or_else(|| config.get("access_token"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let base_url = config.get("base_url").and_then(|v| v.as_str()).unwrap_or(DEFAULT_BASE).trim().trim_end_matches('/').to_string();
        let boards = config_list(config.get("boards"));
        let team_id = config.get("team_id").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
        if token.is_empty() {
            return Err(ConnectorError("miro connector requires token (access token)".to_string()));
        }
        if boards.is_empty() && team_id.is_empty() {
            return Err(ConnectorError("miro connector requires boards (board ids) or team_id".to_string()));
        }
        let client = super::http_client(60)?;
        Ok(Self { token, base_url, boards, team_id, client })
    }

    /// GET one JSON resource; `None` on 404. 429s retry with backoff.
    async fn get_json(&self, path: &str) -> Result<Option<Value>, ConnectorError> {
        let url = format!("{}{}", self.base_url, path);
        for attempt in 0..=MAX_RETRIES {
            let resp = self.client.get(&url).bearer_auth(&self.token).send().await.map_err(|e| ConnectorError(format!("miro request failed: {e}")))?;
            let status = resp.status();
            if status.as_u16() == 429 && attempt < MAX_RETRIES {
                let wait = retry_wait(resp.headers(), attempt);
                tracing::warn!("miro rate limited on {path}; retrying in {wait}s");
                tokio::time::sleep(std::time::Duration::from_secs(wait)).await;
                continue;
            }
            if status.as_u16() == 404 {
                return Ok(None);
            }
            if !status.is_success() {
                let body = resp.text().await.unwrap_or_default();
                return Err(ConnectorError(format!("miro GET {path} failed ({status}): {}", &body[..body.len().min(300)])));
            }
            return resp.json::<Value>().await.map(Some).map_err(|e| ConnectorError(format!("miro json: {e}")));
        }
        Err(ConnectorError(format!("miro GET {path}: still rate limited after {MAX_RETRIES} retries")))
    }

    /// Cursor-paginated list (`items`, `connectors`).
    async fn paged(&self, path: &str) -> Result<Vec<Value>, ConnectorError> {
        let mut out = Vec::new();
        let mut cursor = String::new();
        for _ in 0..MAX_PAGES {
            let sep = if path.contains('?') { '&' } else { '?' };
            let mut page_path = format!("{path}{sep}limit={PAGE_LIMIT}");
            if !cursor.is_empty() {
                page_path.push_str(&format!("&cursor={}", encode_segment(&cursor)));
            }
            let Some(page) = self.get_json(&page_path).await? else { break };
            let data = page.get("data").and_then(|d| d.as_array()).cloned().unwrap_or_default();
            let empty = data.is_empty();
            out.extend(data);
            cursor = page.get("cursor").and_then(|c| c.as_str()).unwrap_or("").to_string();
            if cursor.is_empty() || empty {
                break;
            }
        }
        Ok(out)
    }

    /// Boards in scope: explicit ids (404 = deleted, skipped) + the team's.
    async fn boards_in_scope(&self) -> Result<Vec<Value>, ConnectorError> {
        let mut out = Vec::new();
        for id in &self.boards {
            match self.get_json(&format!("/v2/boards/{}", encode_segment(id))).await? {
                Some(board) => out.push(board),
                None => tracing::warn!("miro board {id} not found (deleted?) — its item will be purged"),
            }
        }
        if !self.team_id.is_empty() {
            out.extend(self.team_boards().await?);
        }
        let mut seen = std::collections::HashSet::new();
        out.retain(|b| seen.insert(str_at(b, "/id").to_string()));
        Ok(out)
    }

    /// Offset-paginated `GET /v2/boards?team_id=…`.
    async fn team_boards(&self) -> Result<Vec<Value>, ConnectorError> {
        let mut out = Vec::new();
        for _ in 0..MAX_PAGES {
            let path = format!("/v2/boards?team_id={}&limit={PAGE_LIMIT}&offset={}", encode_segment(&self.team_id), out.len());
            let Some(page) = self.get_json(&path).await? else { break };
            let data = page.get("data").and_then(|d| d.as_array()).cloned().unwrap_or_default();
            let total = page.get("total").and_then(|t| t.as_u64()).unwrap_or(0) as usize;
            let empty = data.is_empty();
            out.extend(data);
            if empty || out.len() >= total {
                break;
            }
        }
        Ok(out)
    }

    /// Every board in scope. `known` maps board id → the `modifiedAt` of a
    /// fully processed stored item; a match skips the item fetch.
    pub async fn fetch(&self, known: &HashMap<String, String>) -> Result<Vec<BoardFetch>, ConnectorError> {
        let mut out = Vec::new();
        for board in self.boards_in_scope().await? {
            let id = str_at(&board, "/id").to_string();
            if id.is_empty() {
                continue;
            }
            let modified_at = str_at(&board, "/modifiedAt").to_string();
            if !modified_at.is_empty() && known.get(&id) == Some(&modified_at) {
                out.push(BoardFetch::Unchanged { board_id: id, modified_at });
                continue;
            }
            let seg = encode_segment(&id);
            let items = self.paged(&format!("/v2/boards/{seg}/items")).await?;
            let connectors = self.paged(&format!("/v2/boards/{seg}/connectors")).await?;
            let doc = board_doc(&board, &items, &connectors);
            out.push(BoardFetch::Fresh { doc, modified_at });
        }
        Ok(out)
    }
}

fn item_text(item: &Value) -> String {
    let parts: Vec<String> = match str_at(item, "/type") {
        "sticky_note" | "shape" | "text" => vec![plain(str_at(item, "/data/content"))],
        "card" | "app_card" => vec![plain(str_at(item, "/data/title")), plain(str_at(item, "/data/description"))],
        _ => return String::new(),
    };
    parts.into_iter().map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).collect::<Vec<_>>().join(" — ")
}

fn num(v: &Value, ptr: &str) -> f64 {
    v.pointer(ptr).and_then(|x| x.as_f64()).unwrap_or(0.0)
}

/// Items + connectors → the diagram normal form. Pure (no I/O).
pub fn board_diagram(name: &str, description: &str, items: &[Value], connectors: &[Value]) -> Diagram {
    let mut d = Diagram { title: name.to_string(), ..Default::default() };
    if !description.trim().is_empty() {
        // The board description reads before anything drawn on it.
        d.nodes.push(Node { id: "description".into(), text: plain(description), x: f64::MIN, y: f64::MIN, ..Default::default() });
    }
    for item in items {
        let id = str_at(item, "/id").to_string();
        let (x, y) = (num(item, "/position/x"), num(item, "/position/y"));
        let parent = item.pointer("/parent/id").and_then(|p| p.as_str()).map(str::to_string);
        if str_at(item, "/type") == "frame" {
            d.frames.push(Frame { id, title: plain(str_at(item, "/data/title")), parent: None, x, y });
            continue;
        }
        let text = item_text(item);
        if !text.is_empty() {
            d.nodes.push(Node { id, text, frame: parent, x, y, h: num(item, "/geometry/height") });
        }
    }
    for c in connectors {
        let captions: Vec<String> = c
            .get("captions")
            .and_then(|a| a.as_array())
            .map(|a| a.iter().map(|cap| plain(str_at(cap, "/content"))).filter(|s| !s.trim().is_empty()).collect())
            .unwrap_or_default();
        let end = |ptr: &str| c.pointer(ptr).and_then(|x| x.as_str()).map(str::to_string);
        d.edges.push(Edge { from: end("/startItem/id"), to: end("/endItem/id"), label: captions.join(" ") });
    }
    d
}

fn board_doc(board: &Value, items: &[Value], connectors: &[Value]) -> IngestionDoc {
    let id = str_at(board, "/id").to_string();
    let name = str_at(board, "/name");
    let text = crate::diagram::render(&board_diagram(name, str_at(board, "/description"), items, connectors));
    let link = str_at(board, "/viewLink");
    IngestionDoc {
        external_id: external_id(&id),
        title: if name.is_empty() { external_id(&id) } else { name.to_string() },
        text,
        author: str_at(board, "/owner/name").to_string(),
        updated_at: super::parse_dt(str_at(board, "/modifiedAt")),
        source_ref: if link.is_empty() { board_url(&id) } else { link.to_string() },
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    /// Recorded-fixture Miro API: routes by path, counts hits, can answer
    /// the first N requests with 429. Bearer `tok` is required.
    pub(crate) struct MockMiro {
        pub base: String,
        pub hits: Arc<Mutex<Vec<String>>>,
        pub boards: Arc<Mutex<Value>>,
        _task: tokio::task::JoinHandle<()>,
    }

    impl MockMiro {
        pub(crate) fn hits_for(&self, needle: &str) -> usize {
            self.hits.lock().unwrap().iter().filter(|h| h.contains(needle)).count()
        }
    }

    fn fixture_items(cursor: &str) -> Value {
        if cursor.is_empty() {
            json!({"data": [
                {"id": "f1", "type": "frame", "data": {"title": "Discovery"}, "position": {"x": 0, "y": 0}},
                {"id": "s1", "type": "sticky_note", "data": {"content": "<p>Interview <b>users</b></p>"}, "position": {"x": 10, "y": 10}, "geometry": {"height": 40}, "parent": {"id": "f1"}},
                {"id": "img", "type": "image", "data": {}, "position": {"x": 0, "y": 0}}
            ], "cursor": "page-2", "limit": 3, "size": 3})
        } else {
            json!({"data": [
                {"id": "s2", "type": "card", "data": {"title": "Write brief", "description": "<p>one page</p>"}, "position": {"x": 300, "y": 10}, "geometry": {"height": 40}, "parent": {"id": "f1"}},
                {"id": "t1", "type": "text", "data": {"content": "Owner: product"}, "position": {"x": 0, "y": 900}, "geometry": {"height": 20}}
            ], "size": 2})
        }
    }

    fn route(target: &str, boards: &Value) -> (u16, String) {
        let path = target.split('?').next().unwrap_or("");
        let query = target.split_once('?').map(|q| q.1).unwrap_or("");
        let cursor = query.split('&').find_map(|kv| kv.strip_prefix("cursor=")).unwrap_or("");
        if path == "/v2/boards" {
            let data = boards.as_array().cloned().unwrap_or_default();
            return (200, json!({"data": data, "total": data.len(), "offset": 0, "size": data.len()}).to_string());
        }
        if let Some(rest) = path.strip_prefix("/v2/boards/") {
            let id = rest.split('/').next().unwrap_or("");
            let board = boards.as_array().and_then(|a| a.iter().find(|b| b["id"] == id)).cloned();
            let Some(board) = board else { return (404, json!({"message": "not found"}).to_string()) };
            return match rest.split_once('/').map(|x| x.1) {
                None => (200, board.to_string()),
                Some("items") => (200, fixture_items(cursor).to_string()),
                Some("connectors") => (200, json!({"data": [
                    {"id": "c1", "startItem": {"id": "s1"}, "endItem": {"id": "s2"}, "captions": [{"content": "<p>then</p>"}]}
                ]}).to_string()),
                _ => (404, "{}".to_string()),
            };
        }
        (404, "{}".to_string())
    }

    pub(crate) async fn spawn_mock(boards: Value, rate_limited: usize) -> MockMiro {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let hits = Arc::new(Mutex::new(Vec::new()));
        let boards = Arc::new(Mutex::new(boards));
        let limited = Arc::new(AtomicUsize::new(rate_limited));
        let (h, b) = (hits.clone(), boards.clone());
        let task = tokio::spawn(async move {
            loop {
                let Ok((mut s, _)) = listener.accept().await else { break };
                let mut buf = vec![0u8; 8192];
                let n = s.read(&mut buf).await.unwrap_or(0);
                let head = String::from_utf8_lossy(&buf[..n]).into_owned();
                let target = head.split_whitespace().nth(1).unwrap_or("").to_string();
                h.lock().unwrap().push(target.clone());
                let authed = head.lines().any(|l| l.eq_ignore_ascii_case("authorization: bearer tok"));
                let (status, extra, body) = if !authed {
                    (401, String::new(), "{}".to_string())
                } else if limited.load(Ordering::SeqCst) > 0 {
                    limited.fetch_sub(1, Ordering::SeqCst);
                    (429, "Retry-After: 0\r\n".to_string(), "{}".to_string())
                } else {
                    let (st, body) = route(&target, &b.lock().unwrap());
                    (st, String::new(), body)
                };
                let reason = match status { 200 => "OK", 401 => "Unauthorized", 404 => "Not Found", _ => "Too Many Requests" };
                let resp = format!("HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                let _ = s.write_all(resp.as_bytes()).await;
            }
        });
        MockMiro { base: format!("http://{addr}"), hits, boards, _task: task }
    }

    pub(crate) fn board(id: &str, modified: &str) -> Value {
        json!({"id": id, "name": "Research plan", "description": "Q3 research", "modifiedAt": modified, "viewLink": format!("https://miro.com/app/board/{id}/"), "owner": {"name": "Ana"}})
    }

    fn fresh_docs(out: Vec<BoardFetch>) -> Vec<IngestionDoc> {
        out.into_iter().filter_map(|b| match b { BoardFetch::Fresh { doc, .. } => Some(doc), _ => None }).collect()
    }

    #[test]
    fn config_requires_token_and_scope() {
        assert!(MiroConnector::new(&json!({"boards": "b1"})).is_err());
        assert!(MiroConnector::new(&json!({"token": "t"})).is_err());
        assert!(MiroConnector::new(&json!({"token": "t", "boards": ["b1", " ", "b2"]})).unwrap().boards == vec!["b1", "b2"]);
        assert!(MiroConnector::new(&json!({"token": "t", "team_id": "42"})).is_ok());
        assert_eq!(encode_segment("uXjV=a/b"), "uXjV%3Da%2Fb");
    }

    #[tokio::test]
    async fn fetch_paginates_renders_frames_edges_and_retries_429() {
        let mock = spawn_mock(json!([board("b1", "2026-10-20T10:00:00Z")]), 2).await;
        let mc = MiroConnector::new(&json!({"token": "tok", "boards": "b1", "base_url": mock.base})).unwrap();
        let docs = fresh_docs(mc.fetch(&HashMap::new()).await.expect("fetch"));
        assert_eq!(docs.len(), 1);
        let doc = &docs[0];
        assert_eq!((doc.external_id.as_str(), doc.title.as_str(), doc.author.as_str()), ("miro:b1", "Research plan", "Ana"));
        assert_eq!(doc.source_ref, "https://miro.com/app/board/b1/");
        assert_eq!(
            doc.text,
            "# Research plan\n\n- Q3 research\n- Owner: product\n\n## Discovery\n\n- Interview users\n- Write brief — one page\n\n- Interview users → Write brief — one page: then\n"
        );
        assert_eq!(mock.hits_for("cursor=page-2"), 1, "second items page fetched");
        assert_eq!(mock.hits.lock().unwrap().iter().filter(|h| h.starts_with("/v2/boards/b1") && !h.contains("items") && !h.contains("connectors")).count(), 3, "two 429s then success");
        let (sections, _) = crate::markdown::chunk_markdown(&doc.text, "board.md");
        assert!(sections.iter().any(|s| s.path.ends_with("Discovery")), "frames chunk as sections");
    }

    #[tokio::test]
    async fn unchanged_board_skips_items_and_deleted_board_is_dropped() {
        let mock = spawn_mock(json!([board("b1", "2026-10-20T10:00:00Z")]), 0).await;
        let mc = MiroConnector::new(&json!({"token": "tok", "boards": "b1,gone", "base_url": mock.base})).unwrap();
        let known = HashMap::from([("b1".to_string(), "2026-10-20T10:00:00Z".to_string())]);
        let out = mc.fetch(&known).await.expect("fetch");
        assert!(matches!(out.as_slice(), [BoardFetch::Unchanged { board_id, .. }] if board_id == "b1"), "{out:?}");
        assert_eq!(mock.hits_for("/items"), 0, "unchanged board never fetches items");
        let team = MiroConnector::new(&json!({"token": "tok", "team_id": "t1", "base_url": mock.base})).unwrap();
        assert_eq!(team.fetch(&HashMap::new()).await.unwrap().len(), 1, "team scope lists boards");
        let bad = MiroConnector::new(&json!({"token": "wrong", "boards": "b1", "base_url": mock.base})).unwrap();
        assert!(bad.fetch(&HashMap::new()).await.unwrap_err().0.contains("401"));
    }

    #[test]
    fn retry_wait_prefers_retry_after_then_reset_then_backoff() {
        let mut h = reqwest::header::HeaderMap::new();
        assert_eq!(retry_wait(&h, 2), 4);
        h.insert("x-ratelimit-reset", "1".parse().unwrap());
        assert_eq!(retry_wait(&h, 2), 0, "a reset in the past waits 0");
        h.insert("retry-after", "7".parse().unwrap());
        assert_eq!(retry_wait(&h, 2), 7);
        h.insert("retry-after", "9999".parse().unwrap());
        assert_eq!(retry_wait(&h, 2), MAX_WAIT_SECS);
    }
}
