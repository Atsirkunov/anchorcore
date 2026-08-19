use super::{ConnectorError, IngestionDoc};
use chrono::{DateTime, Utc};
use reqwest::Client;
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
        let client = Client::builder().timeout(std::time::Duration::from_secs(60)).build().map_err(|e| ConnectorError(e.to_string()))?;
        let jql = self.jql(since_cursor);
        let mut docs = Vec::new();
        let mut next_cursor = since_cursor.to_string();
        let mut next_page_token: Option<String> = None;
        let mut use_new = true;
        let mut start_at: i64 = 0;
        loop {
            let resp = if use_new {
                let mut params = vec![
                    ("jql", jql.clone()),
                    ("maxResults", "50".to_string()),
                    ("fields", "summary,description,comment,updated,creator,assignee,status".to_string()),
                ];
                if let Some(t) = &next_page_token { params.push(("nextPageToken", t.clone())); }
                let r = client.get(format!("{}/rest/api/3/search/jql", self.base_url))
                    .header("Authorization", self.auth_header())
                    .query(&params)
                    .send().await.map_err(|e| ConnectorError(e.to_string()))?;
                if r.status().as_u16() == 404 || r.status().as_u16() == 410 {
                    use_new = false;
                    continue;
                }
                r
            } else {
                let params = vec![
                    ("jql", jql.clone()),
                    ("startAt", start_at.to_string()),
                    ("maxResults", "50".to_string()),
                    ("fields", "summary,description,comment,updated,creator,assignee,status".to_string()),
                ];
                client.get(format!("{}/rest/api/3/search", self.base_url))
                    .header("Authorization", self.auth_header())
                    .query(&params)
                    .send().await.map_err(|e| ConnectorError(e.to_string()))?
            };
            if !resp.status().is_success() {
                return Err(ConnectorError(format!("Jira {}: {}", resp.status(), resp.text().await.unwrap_or_default())));
            }
            let data: Value = resp.json().await.map_err(|e| ConnectorError(e.to_string()))?;
            for issue in data.get("issues").and_then(|v| v.as_array()).unwrap_or(&vec![]) {
                let fields = issue.get("fields").unwrap_or(&Value::Null);
                let key = issue.get("key").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let summary = fields.get("summary").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let status = fields.get("status").and_then(|v| v.get("name")).and_then(|v| v.as_str()).unwrap_or("").to_string();
                let assignee = fields.get("assignee").and_then(|v| v.get("displayName")).and_then(|v| v.as_str()).unwrap_or("").to_string();
                let desc = fields.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let updated_raw = fields.get("updated").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let updated = DateTime::parse_from_rfc3339(&updated_raw).ok().map(|d| d.with_timezone(&Utc));
                let mut parts = vec![summary.clone(), format!("Status: {}", status), format!("Assignee: {}", assignee), desc];
                if let Some(comments) = fields.get("comment").and_then(|v| v.get("comments")).and_then(|v| v.as_array()) {
                    for c in comments { if let Some(b) = c.get("body").and_then(|v| v.as_str()) { parts.push(b.to_string()); } }
                }
                let text = parts.into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join("\n\n");
                if let Some(dt) = updated {
                    let iso = dt.to_rfc3339();
                    if next_cursor.is_empty() || iso > next_cursor { next_cursor = iso; }
                }
                docs.push(IngestionDoc { external_id: key.clone(), title: if summary.is_empty() { key.clone() } else { summary }, text, author: String::new(), updated_at: updated, source_ref: format!("Jira:{}", key) });
            }
            if use_new {
                let is_last = data.get("isLast").and_then(|v| v.as_bool()).unwrap_or(true);
                if is_last { break; }
                next_page_token = data.get("nextPageToken").and_then(|v| v.as_str()).map(|s| s.to_string());
                if next_page_token.is_none() { break; }
            } else {
                let total = data.get("total").and_then(|v| v.as_i64()).unwrap_or(0);
                let count = data.get("issues").and_then(|v| v.as_array()).map(|a| a.len() as i64).unwrap_or(0);
                if start_at + count >= total { break; }
                start_at += 50;
            }
        }
        Ok((docs, next_cursor))
    }

    pub async fn list_projects(base_url: &str, email: &str, token: &str) -> Result<Vec<(String,String)>, ConnectorError> {
        use base64::{engine::general_purpose::STANDARD, Engine as _};
        let base = base_url.trim_end_matches('/');
        let auth = format!("Basic {}", STANDARD.encode(format!("{}:{}", email, token).as_bytes()));
        let client = Client::builder().timeout(std::time::Duration::from_secs(30)).build().map_err(|e| ConnectorError(e.to_string()))?;
        let mut projects = Vec::new();
        let mut next: Option<String> = None;
        loop {
            let mut params = vec![("maxResults", "50".to_string())];
            if let Some(t) = &next { params.push(("nextPageToken", t.clone())); }
            let resp = client.get(format!("{}/rest/api/3/project/search", base)).header("Authorization", auth.clone()).query(&params).send().await.map_err(|e| ConnectorError(e.to_string()))?;
            if resp.status().as_u16()==404 || resp.status().as_u16()==410 { break; }
            if !resp.status().is_success() { return Err(ConnectorError(format!("{}: {}", resp.status(), resp.text().await.unwrap_or_default()))); }
            let data: Value = resp.json().await.map_err(|e| ConnectorError(e.to_string()))?;
            for p in data.get("values").and_then(|v| v.as_array()).unwrap_or(&vec![]) {
                if let (Some(k), Some(n)) = (p.get("key").and_then(|v| v.as_str()), p.get("name").and_then(|v| v.as_str())) {
                    projects.push((k.to_string(), n.to_string()));
                }
            }
            if data.get("isLast").and_then(|v| v.as_bool()).unwrap_or(true) { break; }
            next = data.get("nextPageToken").and_then(|v| v.as_str()).map(|s| s.to_string());
            if next.is_none() { break; }
        }
        if projects.is_empty() {
            let resp = client.get(format!("{}/rest/api/3/project", base)).header("Authorization", auth).send().await.map_err(|e| ConnectorError(e.to_string()))?;
            if !resp.status().is_success() { return Err(ConnectorError(format!("{}: {}", resp.status(), resp.text().await.unwrap_or_default()))); }
            let data: Value = resp.json().await.map_err(|e| ConnectorError(e.to_string()))?;
            if let Some(arr) = data.as_array() {
                for p in arr { if let (Some(k), Some(n)) = (p.get("key").and_then(|v| v.as_str()), p.get("name").and_then(|v| v.as_str())) { projects.push((k.to_string(), n.to_string())); } }
            }
        }
        Ok(projects)
    }
}
