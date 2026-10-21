use super::{ConnectorError, IngestionDoc};
use serde_json::Value;

pub struct JiraConnector {
    base_url: String,
    email: String,
    token: String,
    project: String,
}

impl JiraConnector {
    pub fn new(config: &serde_json::Value) -> Result<Self, ConnectorError> {
        let base_url = config.get("base_url").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
        let email = config.get("email").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
        let token = config.get("token").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
        let project = config.get("project").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
        if base_url.is_empty() || email.is_empty() || token.is_empty() {
            return Err(ConnectorError("jira connector requires base_url, email, token".to_string()));
        }
        if project.is_empty() {
            return Err(ConnectorError("jira connector requires project".to_string()));
        }
        Ok(Self { base_url: base_url.trim_end_matches('/').to_string(), email, token, project })
    }

    fn auth_header(&self) -> String {
        use base64::{engine::general_purpose::STANDARD, Engine as _};
        let raw = format!("{}:{}", self.email, self.token);
        format!("Basic {}", STANDARD.encode(raw.as_bytes()))
    }

    fn jql(&self, since: &str) -> String {
        let parts: Vec<String> = self.project.split(',').map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).collect();
        let base = if parts.len() > 1 {
            let quoted = parts.iter().map(|p| format!("\"{}\"", p)).collect::<Vec<_>>().join(",");
            format!("project in ({})", quoted)
        } else if let Some(p) = parts.first() {
            format!("project = \"{}\"", p)
        } else {
            format!("project = \"{}\"", self.project)
        };
        if since.is_empty() { format!("{} ORDER BY updated ASC", base) } else { format!("{} AND updated > \"{}\" ORDER BY updated ASC", base, since) }
    }

    pub async fn fetch(&self, since_cursor: &str) -> Result<(Vec<IngestionDoc>, String), ConnectorError> {
        // R15.5: pinned to `search/jql` (Atlassian removed the legacy `search`
        // `startAt` API — the old dual-path `use_new` fork is gone).
        let client = super::http_client(60)?;
        let jql = self.jql(since_cursor);
        let mut docs = Vec::new();
        let mut next_cursor = since_cursor.to_string();
        let mut next_page_token: Option<String> = None;
        loop {
            let mut params = vec![
                ("jql", jql.clone()),
                ("maxResults", "50".to_string()),
                ("fields", "summary,description,comment,updated,creator,assignee,status".to_string()),
            ];
            if let Some(t) = &next_page_token { params.push(("nextPageToken", t.clone())); }
            let resp = client.get(format!("{}/rest/api/3/search/jql", self.base_url))
                .header("Authorization", self.auth_header())
                .query(&params)
                .send().await.map_err(|e| ConnectorError(e.to_string()))?;
            if !resp.status().is_success() {
                return Err(ConnectorError(format!("Jira {}: {}", resp.status(), resp.text().await.unwrap_or_default())));
            }
            let data: Value = resp.json().await.map_err(|e| ConnectorError(e.to_string()))?;
            for issue in data.get("issues").and_then(|v| v.as_array()).unwrap_or(&vec![]) {
                parse_issue(issue, &mut docs, &mut next_cursor);
            }
            if super::next_jql_token(&data, &mut next_page_token) {
                break;
            }
        }
        Ok((docs, next_cursor))
    }
}

/// R43.5: Jira API v3 returns rich text as ADF (Atlassian Document Format)
/// JSON objects, not strings — `.as_str()` on them yields nothing, so only
/// summary/status/assignee were indexed. Flatten generically by JSON shape:
/// walk `content` arrays, collect `text` nodes; block siblings join with
/// newlines, inline siblings (paragraph/heading children) join directly.
/// Plain strings (older payloads) pass through untouched.
fn adf_text(value: &Value) -> String {
    if let Some(s) = value.as_str() {
        return s.to_string();
    }
    adf_node_text(value)
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn adf_node_text(node: &Value) -> String {
    if let Some(s) = node.as_str() {
        return s.to_string();
    }
    let typ = node.get("type").and_then(|v| v.as_str()).unwrap_or("");
    // Inline leaf nodes carry their payload outside `content`.
    if typ == "text" {
        return node.get("text").and_then(|v| v.as_str()).unwrap_or("").to_string();
    }
    if typ == "hardBreak" {
        return "\n".to_string();
    }
    if typ == "mention" {
        return node.get("attrs").and_then(|a| a.get("text")).and_then(|v| v.as_str()).unwrap_or("").to_string();
    }
    if typ == "emoji" {
        return node.get("attrs").and_then(|a| a.get("shortName")).and_then(|v| v.as_str()).unwrap_or("").to_string();
    }
    if typ == "media" {
        return node.get("attrs").and_then(|a| a.get("alt")).and_then(|v| v.as_str()).unwrap_or("").to_string();
    }
    if typ == "status" {
        return node.get("attrs").and_then(|a| a.get("text")).and_then(|v| v.as_str()).unwrap_or("").to_string();
    }
    if typ == "inlineCard" {
        return node.get("attrs").and_then(|a| a.get("url")).and_then(|v| v.as_str()).unwrap_or("").to_string();
    }
    let empty = vec![];
    let kids = node.get("content").and_then(|v| v.as_array()).unwrap_or(&empty);
    // `is_empty` (not trim): a lone `hardBreak` "\n" part must survive.
    let parts: Vec<String> = kids.iter().map(adf_node_text).filter(|s| !s.is_empty()).collect();
    // Inline containers join directly; every other level (doc, lists,
    // tables, panels…) separates block children with newlines.
    let sep = if typ == "paragraph" || typ == "heading" { "" } else { "\n" };
    parts.join(sep)
}

fn parse_issue(issue: &Value, docs: &mut Vec<IngestionDoc>, next_cursor: &mut String) {
    let fields = issue.get("fields").unwrap_or(&Value::Null);
    let key = issue.get("key").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let summary = fields.get("summary").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let status = fields.get("status").and_then(|v| v.get("name")).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let assignee = fields.get("assignee").and_then(|v| v.get("displayName")).and_then(|v| v.as_str()).unwrap_or("").to_string();
    // R43.5: description is ADF JSON on API v3 (already in the fetched
    // payload — `fields=...description,comment...` in `fetch`).
    let desc = fields.get("description").map(adf_text).unwrap_or_default();
    let updated_raw = fields.get("updated").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let updated = super::parse_dt(&updated_raw);
    let mut parts = vec![summary.clone(), format!("Status: {}", status), format!("Assignee: {}", assignee), desc];
    if let Some(comments) = fields.get("comment").and_then(|v| v.get("comments")).and_then(|v| v.as_array()) {
        for c in comments {
            // R43.5: comment bodies are ADF too; no new API call needed.
            let body = c.get("body").map(adf_text).unwrap_or_default();
            if !body.is_empty() {
                parts.push(body);
            }
        }
    }
    super::push_doc(docs, next_cursor, &key, &summary, parts, String::new(), updated, "Jira");
}

fn collect_projects(data: &Value, projects: &mut Vec<(String, String)>) {
    // Handles both `project/search` (`values` array) and legacy `project` (bare array).
    let arr = data.get("values").and_then(|v| v.as_array()).or_else(|| data.as_array());
    for p in arr.unwrap_or(&vec![]) {
        if let (Some(k), Some(n)) = (p.get("key").and_then(|v| v.as_str()), p.get("name").and_then(|v| v.as_str())) {
            projects.push((k.to_string(), n.to_string()));
        }
    }
}

impl JiraConnector {
    pub async fn list_projects(base_url: &str, email: &str, token: &str) -> Result<Vec<(String,String)>, ConnectorError> {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    let base = base_url.trim_end_matches('/');
    let auth = format!("Basic {}", STANDARD.encode(format!("{}:{}", email, token).as_bytes()));
    let client = super::http_client(30)?;
    let mut projects = Vec::new();
    let mut next: Option<String> = None;
    loop {
        let mut params = vec![("maxResults", "50".to_string())];
        if let Some(t) = &next { params.push(("nextPageToken", t.clone())); }
        let resp = client.get(format!("{}/rest/api/3/project/search", base)).header("Authorization", auth.clone()).query(&params).send().await.map_err(|e| ConnectorError(e.to_string()))?;
        if resp.status().as_u16()==404 || resp.status().as_u16()==410 { break; }
        if !resp.status().is_success() { return Err(ConnectorError(format!("{}: {}", resp.status(), resp.text().await.unwrap_or_default()))); }
        let data: Value = resp.json().await.map_err(|e| ConnectorError(e.to_string()))?;
        collect_projects(&data, &mut projects);
        if super::next_jql_token(&data, &mut next) { break; }
    }
    if projects.is_empty() {
        let resp = client.get(format!("{}/rest/api/3/project", base)).header("Authorization", auth).send().await.map_err(|e| ConnectorError(e.to_string()))?;
        if !resp.status().is_success() { return Err(ConnectorError(format!("{}: {}", resp.status(), resp.text().await.unwrap_or_default()))); }
        let data: Value = resp.json().await.map_err(|e| ConnectorError(e.to_string()))?;
        collect_projects(&data, &mut projects);
    }
    Ok(projects)
}
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn adf_issue() -> Value {
        json!({
            "key": "PROJ-1",
            "fields": {
                "summary": "Login fails on Safari",
                "status": {"name": "In Progress"},
                "assignee": {"displayName": "Ada"},
                "updated": "2026-10-20T10:00:00Z",
                "description": {
                    "type": "doc", "version": 1,
                    "content": [
                        {"type": "paragraph", "content": [
                            {"type": "text", "text": "Steps to reproduce:"}
                        ]},
                        {"type": "bulletList", "content": [
                            {"type": "listItem", "content": [
                                {"type": "paragraph", "content": [{"type": "text", "text": "Open Safari"}]}
                            ]},
                            {"type": "listItem", "content": [
                                {"type": "paragraph", "content": [{"type": "text", "text": "Click login"}]}
                            ]}
                        ]}
                    ]
                },
                "comment": {"comments": [
                    {"body": {"type": "doc", "version": 1, "content": [
                        {"type": "paragraph", "content": [{"type": "text", "text": "Confirmed on 17.4"}]}
                    ]}}
                ]}
            }
        })
    }

    #[test]
    fn adf_description_and_comment_flattened() {
        let mut docs = Vec::new();
        let mut cursor = String::new();
        parse_issue(&adf_issue(), &mut docs, &mut cursor);
        assert_eq!(docs.len(), 1);
        let text = &docs[0].text;
        assert!(text.contains("Login fails on Safari"), "summary: {text}");
        assert!(text.contains("Status: In Progress"), "status: {text}");
        assert!(text.contains("Assignee: Ada"), "assignee: {text}");
        assert!(text.contains("Steps to reproduce:"), "desc para: {text}");
        assert!(text.contains("Open Safari"), "desc bullet 1: {text}");
        assert!(text.contains("Click login"), "desc bullet 2: {text}");
        assert!(text.contains("Confirmed on 17.4"), "comment: {text}");
        assert_eq!(docs[0].title, "Login fails on Safari");
        assert_eq!(docs[0].source_ref, "Jira:PROJ-1");
        assert!(!cursor.is_empty(), "cursor must advance on updated");
    }

    #[test]
    fn plain_string_description_passthrough() {
        // Older payloads (or already-rendered text) stay byte-identical.
        let issue = json!({
            "key": "PROJ-2",
            "fields": {
                "summary": "S", "status": {"name": "Done"},
                "assignee": {"displayName": "Bo"},
                "description": "plain body",
                "comment": {"comments": [{"body": "plain comment"}]}
            }
        });
        let mut docs = Vec::new();
        let mut cursor = String::new();
        parse_issue(&issue, &mut docs, &mut cursor);
        assert!(docs[0].text.contains("plain body"), "{}", docs[0].text);
        assert!(docs[0].text.contains("plain comment"), "{}", docs[0].text);
    }

    #[test]
    fn adf_inline_nodes_and_nulls() {
        assert_eq!(adf_text(&Value::Null), "");
        assert_eq!(adf_text(&json!("x")), "x");
        // hardBreak splits lines, mention resolves via attrs.text
        let doc = json!({"type": "doc", "content": [
            {"type": "paragraph", "content": [
                {"type": "text", "text": "hi "},
                {"type": "mention", "attrs": {"text": "@Ada"}},
                {"type": "hardBreak"},
                {"type": "text", "text": "bye"}
            ]}
        ]});
        assert_eq!(adf_text(&doc), "hi @Ada\nbye");
        // status macros keep their label; smart-link cards keep their URL
        let doc = json!({"type": "doc", "content": [
            {"type": "paragraph", "content": [
                {"type": "status", "attrs": {"text": "IN REVIEW"}},
                {"type": "text", "text": " see "},
                {"type": "inlineCard", "attrs": {"url": "https://jira.example/browse/PROJ-9"}}
            ]}
        ]});
        assert_eq!(adf_text(&doc), "IN REVIEW see https://jira.example/browse/PROJ-9");
        // null description + absent comments → header parts only
        let issue = json!({
            "key": "PROJ-3",
            "fields": {"summary": "S", "status": {"name": "Todo"}, "description": Value::Null}
        });
        let mut docs = Vec::new();
        let mut cursor = String::new();
        parse_issue(&issue, &mut docs, &mut cursor);
        assert!(docs[0].text.contains("Status: Todo"), "{}", docs[0].text);
    }
}
