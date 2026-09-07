use super::{ConnectorError, IngestionDoc};
use serde_json::Value;

/// Generic REST API connector — the user maps their API's JSON fields onto
/// `IngestionDoc` (external_id/title/text/author/updated_at) via JSON pointers.
///
/// Flat string config (fits `Record<string,string>` from the UI + keychain secrets):
/// - `base_url` (required), `list_path` (required, e.g. `/api/v1/issues`;
///   a full `http...` URL is used as-is), `method` (`GET` default, or `POST`)
/// - auth: `auth_mode` (`none`|`bearer`|`basic`|`header`, default `none`);
///   bearer → `token`, basic → `email`+`password`, header → `header_name`+`api_key`
///   (`token`/`api_key`/`password` are keychain-backed secrets, never in DB)
/// - `items_path`: JSON pointer to the item array (`/data`, `/issues`, ...;
///   empty = body is the array, else auto-try `/data`, `/items`, `/results`)
/// - field map (JSON pointers; `a.b` shorthand accepted, `/a/b` verbatim):
///   `map_id` (default `/id`), `map_title` (default `/title`),
///   `map_text` (comma-separated pointers joined with blank lines, default `/body`),
///   `map_author` (default `/author`), `map_updated` (default `/updated_at`,
///   RFC3339 string or unix-epoch number)
/// - pagination `page_mode` (`none`|`token`|`page`|`offset`, default `none`):
///   token → `page_param` (default `pageToken`) + `next_token_path`
///   (default `/nextPageToken`); page → `page_param`/`page_size_param`/`page_size`/
///   `page_start`; offset → `offset_param`/`limit_param`/`page_size`.
///   `max_pages` bounds the loop (default 20, clamped 1..=200).
/// - incremental `since_param` (optional): sent with the sync cursor value.
/// - `POST` bodies merge `body_json` (raw JSON string) with the page params.
pub struct RestConnector {
    base_url: String,
    list_path: String,
    method: String,
    auth_mode: String,
    email: String,
    token: String,
    password: String,
    api_key: String,
    header_name: String,
    items_path: String,
    map_id: String,
    map_title: String,
    map_text: String,
    map_author: String,
    map_updated: String,
    page_mode: String,
    page_param: String,
    next_token_path: String,
    page_size_param: String,
    page_size: i64,
    page_start: i64,
    offset_param: String,
    limit_param: String,
    max_pages: u64,
    since_param: String,
    body_json: String,
    ref_prefix: String,
}

fn cfg_str(config: &Value, key: &str, default: &str) -> String {
    config.get(key).and_then(|v| v.as_str()).unwrap_or(default).trim().to_string()
}

fn cfg_i64(config: &Value, key: &str, default: i64) -> i64 {
    config
        .get(key)
        .and_then(|v| v.as_str().unwrap_or("").parse::<i64>().ok().or_else(|| v.as_i64()))
        .unwrap_or(default)
}

impl RestConnector {
    pub fn new(config: &Value) -> Result<Self, ConnectorError> {
        let base_url = cfg_str(config, "base_url", "").trim_end_matches('/').to_string();
        let list_path = cfg_str(config, "list_path", "");
        if base_url.is_empty() {
            return Err(ConnectorError("rest connector requires base_url".to_string()));
        }
        if list_path.is_empty() {
            return Err(ConnectorError("rest connector requires list_path".to_string()));
        }
        let auth_mode = cfg_str(config, "auth_mode", "none").to_lowercase();
        if !["none", "bearer", "basic", "header"].contains(&auth_mode.as_str()) {
            return Err(ConnectorError("rest auth_mode must be none|bearer|basic|header".to_string()));
        }
        if auth_mode == "bearer" && cfg_str(config, "token", "").is_empty() {
            return Err(ConnectorError("rest bearer auth requires token".to_string()));
        }
        if auth_mode == "header" && cfg_str(config, "api_key", "").is_empty() {
            return Err(ConnectorError("rest header auth requires api_key".to_string()));
        }
        let page_mode = cfg_str(config, "page_mode", "none").to_lowercase();
        if !["none", "token", "page", "offset"].contains(&page_mode.as_str()) {
            return Err(ConnectorError("rest page_mode must be none|token|page|offset".to_string()));
        }
        let method = cfg_str(config, "method", "GET").to_uppercase();
        if method != "GET" && method != "POST" {
            return Err(ConnectorError("rest method must be GET|POST".to_string()));
        }
        Ok(Self {
            base_url,
            list_path,
            method,
            auth_mode,
            email: cfg_str(config, "email", ""),
            token: cfg_str(config, "token", ""),
            password: cfg_str(config, "password", ""),
            api_key: cfg_str(config, "api_key", ""),
            header_name: cfg_str(config, "header_name", "X-Api-Key"),
            items_path: cfg_str(config, "items_path", ""),
            map_id: cfg_str(config, "map_id", "/id"),
            map_title: cfg_str(config, "map_title", "/title"),
            map_text: cfg_str(config, "map_text", "/body"),
            map_author: cfg_str(config, "map_author", "/author"),
            map_updated: cfg_str(config, "map_updated", "/updated_at"),
            page_mode,
            page_param: cfg_str(config, "page_param", ""),
            next_token_path: cfg_str(config, "next_token_path", "/nextPageToken"),
            page_size_param: cfg_str(config, "page_size_param", ""),
            page_size: cfg_i64(config, "page_size", 50).clamp(1, 1000),
            page_start: cfg_i64(config, "page_start", 1),
            offset_param: cfg_str(config, "offset_param", ""),
            limit_param: cfg_str(config, "limit_param", ""),
            max_pages: cfg_i64(config, "max_pages", 20).clamp(1, 200) as u64,
            since_param: cfg_str(config, "since_param", ""),
            body_json: cfg_str(config, "body_json", ""),
            ref_prefix: cfg_str(config, "ref_prefix", "rest"),
        })
    }

    fn url(&self) -> String {
        if self.list_path.starts_with("http://") || self.list_path.starts_with("https://") {
            return self.list_path.clone();
        }
        format!("{}{}", self.base_url, self.list_path)
    }

    /// Query params for the current page (token/page/offset modes + since cursor).
    fn page_params(&self, state: &PageState, since: &str) -> Vec<(String, String)> {
        let mut params = Vec::new();
        if !self.since_param.is_empty() && !since.is_empty() {
            params.push((self.since_param.clone(), since.to_string()));
        }
        match self.page_mode.as_str() {
            "token" => {
                let name = if self.page_param.is_empty() { "pageToken".to_string() } else { self.page_param.clone() };
                if let Some(t) = &state.token {
                    params.push((name, t.clone()));
                }
            }
            "page" => {
                let name = if self.page_param.is_empty() { "page".to_string() } else { self.page_param.clone() };
                let size_name = if self.page_size_param.is_empty() { "per_page".to_string() } else { self.page_size_param.clone() };
                params.push((name, state.page.to_string()));
                params.push((size_name, self.page_size.to_string()));
            }
            "offset" => {
                let name = if self.offset_param.is_empty() { "offset".to_string() } else { self.offset_param.clone() };
                let limit_name = if self.limit_param.is_empty() { "limit".to_string() } else { self.limit_param.clone() };
                params.push((name, state.offset.to_string()));
                params.push((limit_name, self.page_size.to_string()));
            }
            _ => {}
        }
        params
    }

    async fn do_request(&self, client: &reqwest::Client, params: &[(String, String)]) -> Result<Value, ConnectorError> {
        let mut req = if self.method == "POST" { client.post(self.url()) } else { client.get(self.url()) };
        req = apply_auth(req, self);
        if self.method == "POST" {
            let mut body = serde_json::from_str::<Value>(&self.body_json).unwrap_or(serde_json::json!({}));
            if !body.is_object() {
                body = serde_json::json!({});
            }
            let obj = body.as_object_mut().unwrap();
            for (k, v) in params {
                obj.insert(k.clone(), Value::String(v.clone()));
            }
            req = req.json(&body);
        } else {
            req = req.query(params);
        }
        let resp = req.send().await.map_err(|e| ConnectorError(format!("REST request failed: {}", e)))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let snippet: String = resp.text().await.unwrap_or_default().chars().take(300).collect();
            return Err(ConnectorError(format!("REST {}: {}", status, snippet)));
        }
        resp.json::<Value>().await.map_err(|e| ConnectorError(format!("REST invalid JSON: {}", e)))
    }

    pub async fn fetch(&self, since_cursor: &str) -> Result<(Vec<IngestionDoc>, String), ConnectorError> {
        let client = super::http_client(60)?;
        let mut docs = Vec::new();
        let mut next_cursor = since_cursor.to_string();
        let mut state = PageState::new(self.page_start);
        for _ in 0..self.max_pages {
            let params = self.page_params(&state, since_cursor);
            let data = self.do_request(&client, &params).await?;
            let items = find_items(&data, &self.items_path);
            if items.is_empty() {
                break;
            }
            let mut n = 0;
            for (i, item) in items.iter().enumerate() {
                if let Some(doc) = map_item(item, self, i) {
                    super::bump_cursor(&mut next_cursor, doc.updated_at);
                    docs.push(doc);
                    n += 1;
                }
            }
            state.count = n;
            if !state.advance(&data, self) {
                break;
            }
        }
        Ok((docs, next_cursor))
    }
}

/// `/a/b` used verbatim (RFC6901); `a.b` shorthand converted with `~`/`/` escaping.
fn normalize_pointer(raw: &str) -> String {
    let s = raw.trim();
    if s.is_empty() || s.starts_with('/') {
        return s.to_string();
    }
    let mut out = String::new();
    for seg in s.split('.') {
        out.push('/');
        out.push_str(&seg.replace('~', "~0").replace('/', "~1"));
    }
    out
}

/// Scalar leaf as string (string/number/bool); objects/arrays/missing → "".
fn extract_scalar(item: &Value, pointer: &str) -> String {
    let p = normalize_pointer(pointer);
    if p.is_empty() {
        return String::new();
    }
    match item.pointer(&p) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Bool(b)) => b.to_string(),
        _ => String::new(),
    }
}

fn extract_updated(item: &Value, pointer: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    let p = normalize_pointer(pointer);
    if p.is_empty() {
        return None;
    }
    match item.pointer(&p) {
        Some(Value::String(s)) => super::parse_dt(s),
        Some(Value::Number(n)) => n.as_i64().and_then(|e| chrono::DateTime::from_timestamp(e, 0)),
        _ => None,
    }
}

fn split_pointers(raw: &str) -> Vec<String> {
    raw.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
}

/// Locate the item array: explicit pointer, else body-as-array, else common wrappers.
fn find_items<'a>(body: &'a Value, items_path: &str) -> Vec<&'a Value> {
    let p = normalize_pointer(items_path);
    if !p.is_empty() {
        return body.pointer(&p).and_then(|v| v.as_array()).map(|a| a.iter().collect()).unwrap_or_default();
    }
    if let Some(a) = body.as_array() {
        return a.iter().collect();
    }
    for key in ["/data", "/items", "/results"] {
        if let Some(a) = body.pointer(key).and_then(|v| v.as_array()) {
            return a.iter().collect();
        }
    }
    Vec::new()
}

fn stable_fallback_id(title: &str, text: &str) -> String {
    let digest = crate::hashing::content_hash(&format!("{}\n{}", title, text));
    format!("rest-{}", digest.chars().take(16).collect::<String>())
}

/// Map one API item to an ingestion doc. `None` skips content-less items
/// (title+text both empty). Empty ids get a stable content-hash fallback so
/// re-syncs upsert instead of duplicating (`external_id` is the upsert key).
fn map_item(item: &Value, cfg: &RestConnector, index: usize) -> Option<IngestionDoc> {
    let title = extract_scalar(item, &cfg.map_title);
    let parts: Vec<String> = split_pointers(&cfg.map_text)
        .iter()
        .map(|p| extract_scalar(item, p))
        .filter(|s| !s.is_empty())
        .collect();
    let text = super::join_parts(parts);
    if title.is_empty() && text.is_empty() {
        return None;
    }
    let mut id = extract_scalar(item, &cfg.map_id);
    if id.is_empty() {
        id = item
            .get("id")
            .or_else(|| item.get("key"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| stable_fallback_id(&title, &text));
    }
    let author = extract_scalar(item, &cfg.map_author);
    let updated = extract_updated(item, &cfg.map_updated);
    let prefix = if cfg.ref_prefix.is_empty() { "rest".to_string() } else { cfg.ref_prefix.clone() };
    let _ = index;
    Some(IngestionDoc {
        source_ref: format!("{}:{}", prefix, id),
        external_id: id,
        title: if title.is_empty() { format!("rest item {}", index) } else { title },
        text,
        author,
        updated_at: updated,
    })
}

/// Attach credentials: bearer/basic/custom header. Pure builder step (no I/O).
fn apply_auth(builder: reqwest::RequestBuilder, cfg: &RestConnector) -> reqwest::RequestBuilder {
    match cfg.auth_mode.as_str() {
        "bearer" => builder.header("Authorization", format!("Bearer {}", cfg.token)),
        "basic" => builder.basic_auth(cfg.email.clone(), Some(cfg.password.clone())),
        "header" => builder.header(cfg.header_name.clone(), cfg.api_key.clone()),
        _ => builder,
    }
}

struct PageState {
    page: i64,
    offset: i64,
    token: Option<String>,
    count: usize,
}

impl PageState {
    fn new(start: i64) -> Self {
        Self { page: start, offset: 0, token: None, count: 0 }
    }

    /// Advance to the next page. Returns false when the list is exhausted.
    fn advance(&mut self, body: &Value, cfg: &RestConnector) -> bool {
        match cfg.page_mode.as_str() {
            "token" => {
                let next = body
                    .pointer(&normalize_pointer(&cfg.next_token_path))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .filter(|s| !s.is_empty());
                self.token = next;
                self.token.is_some()
            }
            "page" => {
                self.page += 1;
                self.count as i64 >= cfg.page_size
            }
            "offset" => {
                self.offset += cfg.page_size;
                self.count as i64 >= cfg.page_size
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(pairs: &[(&str, &str)]) -> Value {
        let mut m = serde_json::Map::new();
        for (k, v) in pairs {
            m.insert(k.to_string(), Value::String(v.to_string()));
        }
        Value::Object(m)
    }

    #[test]
    fn requires_base_and_path() {
        assert!(RestConnector::new(&cfg(&[])).is_err());
        assert!(RestConnector::new(&cfg(&[("base_url", "https://x.example")])).is_err());
        let ok = cfg(&[("base_url", "https://x.example/"), ("list_path", "/v1/t")]);
        let c = RestConnector::new(&ok).unwrap();
        assert_eq!(c.url(), "https://x.example/v1/t");
    }

    #[test]
    fn rejects_bad_enums_and_missing_secrets() {
        let base = [("base_url", "https://x.example"), ("list_path", "/t")];
        assert!(RestConnector::new(&cfg(&[base[0], base[1], ("auth_mode", "oauth")])).is_err());
        assert!(RestConnector::new(&cfg(&[base[0], base[1], ("auth_mode", "bearer")])).is_err());
        assert!(RestConnector::new(&cfg(&[base[0], base[1], ("page_mode", "cursor")])).is_err());
        assert!(RestConnector::new(&cfg(&[base[0], base[1], ("method", "DELETE")])).is_err());
    }

    #[test]
    fn pointer_shorthand() {
        assert_eq!(normalize_pointer("/fields/summary"), "/fields/summary");
        assert_eq!(normalize_pointer("fields.summary"), "/fields/summary");
        assert_eq!(normalize_pointer("title"), "/title");
    }

    #[test]
    fn extracts_and_joins_text_parts() {
        let item = serde_json::json!({"title": "T", "a": "one", "b": {"c": "two"}, "n": 42});
        assert_eq!(extract_scalar(&item, "title"), "T");
        assert_eq!(extract_scalar(&item, "b.c"), "two");
        assert_eq!(extract_scalar(&item, "n"), "42");
        assert_eq!(extract_scalar(&item, "missing"), "");
    }

    #[test]
    fn maps_item_with_defaults() {
        let c = RestConnector::new(&cfg(&[("base_url", "https://x.example"), ("list_path", "/t")])).unwrap();
        let item = serde_json::json!({"id": "A-1", "title": "Hello", "body": "World", "author": "Ann",
            "updated_at": "2026-01-02T03:04:05Z"});
        let doc = map_item(&item, &c, 0).unwrap();
        assert_eq!(doc.external_id, "A-1");
        assert_eq!(doc.source_ref, "rest:A-1");
        assert_eq!(doc.text, "World");
        assert!(doc.updated_at.is_some());
    }

    #[test]
    fn skips_empty_and_stabilizes_missing_id() {
        let c = RestConnector::new(&cfg(&[("base_url", "https://x.example"), ("list_path", "/t")])).unwrap();
        assert!(map_item(&serde_json::json!({"id": "x"}), &c, 0).is_none());
        let item = serde_json::json!({"title": "T", "body": "B"});
        let a = map_item(&item, &c, 3).unwrap();
        let b = map_item(&item, &c, 3).unwrap();
        assert_eq!(a.external_id, b.external_id, "fallback id must be stable across syncs");
        assert!(a.external_id.starts_with("rest-"));
    }

    #[test]
    fn finds_arrays_and_wrappers() {
        let arr = serde_json::json!([{"id": 1}]);
        assert_eq!(find_items(&arr, "").len(), 1);
        let wrapped = serde_json::json!({"data": [{"id": 1}, {"id": 2}]});
        assert_eq!(find_items(&wrapped, "").len(), 2);
        assert_eq!(find_items(&wrapped, "/data").len(), 2);
        assert!(find_items(&wrapped, "/nope").is_empty());
    }

    #[test]
    fn pagination_params_per_mode() {
        let mk = |pairs: &[(&str, &str)]| {
            let mut v: Vec<(&str, &str)> = vec![("base_url", "https://x.example"), ("list_path", "/t")];
            v.extend_from_slice(pairs);
            RestConnector::new(&cfg(&v)).unwrap()
        };
        let st = PageState::new(1);
        assert!(mk(&[]).page_params(&st, "").is_empty());
        let token_c = mk(&[("page_mode", "token")]);
        assert!(token_c.page_params(&st, "").is_empty(), "first token page sends no token");
        let page_c = mk(&[("page_mode", "page")]);
        let p = page_c.page_params(&st, "");
        assert!(p.contains(&("page".to_string(), "1".to_string())));
        let off_c = mk(&[("page_mode", "offset")]);
        let o = off_c.page_params(&st, "");
        assert!(o.contains(&("offset".to_string(), "0".to_string())));
        let since_c = mk(&[("since_param", "updated_since")]);
        let s = since_c.page_params(&st, "2026-01-01T00:00:00Z");
        assert!(s.contains(&("updated_since".to_string(), "2026-01-01T00:00:00Z".to_string())));
    }

    #[test]
    fn page_state_advance() {
        let mk = |mode: &str| {
            RestConnector::new(&cfg(&[
                ("base_url", "https://x.example"), ("list_path", "/t"),
                ("page_mode", mode), ("page_size", "2"),
            ]))
            .unwrap()
        };
        let full = serde_json::json!([1, 2]);
        let mut st = PageState::new(1);
        st.count = 2;
        assert!(st.advance(&full, &mk("page")));
        st.count = 1;
        assert!(!st.advance(&full, &mk("page")), "short page ends page mode");
        let tok = serde_json::json!({"nextPageToken": "abc"});
        let mut st2 = PageState::new(1);
        st2.count = 0;
        let tc = mk("token");
        assert!(st2.advance(&tok, &tc));
        assert!(!st2.advance(&serde_json::json!({}), &tc), "missing token ends token mode");
    }
}
