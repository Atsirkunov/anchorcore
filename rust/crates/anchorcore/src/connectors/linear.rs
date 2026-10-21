use super::{ConnectorError, IngestionDoc};
use reqwest::Client;
use serde_json::Value;

/// Linear connector — GraphQL `api.linear.app/graphql` with Bearer API key.
/// Mirrors `backend/app/connectors/jira.py` shape but for Linear teams/issues.
///
/// Config: `api_key` (or `token`) + `team` (Linear team key, e.g. "ENG", comma-separated for multiple)
/// `base_url` optional, defaults to https://api.linear.app/graphql.
/// Incremental via `updatedAt > since_cursor` (RFC3339).
pub struct LinearConnector {
    api_key: String,
    team: String,
    base_url: String,
}

fn parse_issue(node: &Value, docs: &mut Vec<IngestionDoc>, next_cursor: &mut String) {
    let id = node.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let identifier = node.get("identifier").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let title = node.get("title").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let desc = node.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let state = node.get("state").and_then(|v| v.get("name")).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let assignee = node.get("assignee").and_then(|v| v.get("name")).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let creator = node.get("creator").and_then(|v| v.get("name")).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let updated_raw = node.get("updatedAt").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let updated = super::parse_dt(&updated_raw);
    let mut parts = vec![title.clone(), format!("State: {}", state), format!("Assignee: {}", assignee), desc.clone()];
    if let Some(comments) = node.get("comments").and_then(|v| v.get("nodes")).and_then(|v| v.as_array()) {
        for c in comments {
            if let Some(b) = c.get("body").and_then(|v| v.as_str()) {
                if !b.is_empty() { parts.push(b.to_string()); }
            }
        }
    }
    let external_id = if identifier.is_empty() { id } else { identifier };
    let author = if !creator.is_empty() { creator } else { assignee.clone() };
    super::push_doc(docs, next_cursor, &external_id, &title, parts, author, updated, "Linear");
}

/// Escape a value interpolated into a GraphQL string literal.
fn gql_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Build the one-team issues query for a page. Pure (no I/O) so tests can
/// assert the exact GraphQL text without network.
fn build_issues_query(team_key: &str, after: &str, since_cursor: &str) -> String {
    // GraphQL: filter by team key and updatedAt > since.
    // Single query that works for both single and multiple teams — filter by team key.
    // Use `issues` root query with filter; fallback to `team.issues` if needed.
    let filter_since = if since_cursor.is_empty() {
        String::new()
    } else {
        format!(", updatedAt: {{ gt: \"{}\" }}", gql_escape(since_cursor))
    };
    let after_clause = if after.is_empty() {
        String::new()
    } else {
        format!(", after: \"{}\"", gql_escape(after))
    };
    // NOTE: raw string — backslashes are literal here, so the GraphQL string
    // quotes are plain `"`; a `\"` would reach Linear and fail parsing.
    format!(
        r#"query {{
          issues(first: 50{} filter: {{ team: {{ key: {{ eq: "{}" }} }}{} }}) {{
            nodes {{
              id
              identifier
              title
              description
              updatedAt
              creator {{ name }}
              assignee {{ name }}
              state {{ name }}
              team {{ key name }}
              comments {{ nodes {{ body }} }}
            }}
            pageInfo {{ hasNextPage endCursor }}
          }}
        }}"#,
        after_clause,
        gql_escape(team_key),
        filter_since
    )
}

impl LinearConnector {
    pub fn new(config: &Value) -> Result<Self, ConnectorError> {
        let api_key = config
            .get("api_key")
            .or_else(|| config.get("token"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let team = config
            .get("team")
            .or_else(|| config.get("project"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let base_url = config
            .get("base_url")
            .and_then(|v| v.as_str())
            .unwrap_or("https://api.linear.app/graphql")
            .trim()
            .trim_end_matches('/')
            .to_string();

        if api_key.is_empty() {
            return Err(ConnectorError("linear connector requires api_key (or token)".to_string()));
        }
        if team.is_empty() {
            return Err(ConnectorError("linear connector requires team (Linear team key, e.g. ENG)".to_string()));
        }
        Ok(Self { api_key, team, base_url })
    }

    fn teams(&self) -> Vec<String> {
        self.team
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    }

    pub async fn fetch(&self, since_cursor: &str) -> Result<(Vec<IngestionDoc>, String), ConnectorError> {
        let client = super::http_client(60)?;

        let teams = self.teams();
        let mut all_docs = Vec::new();
        let mut next_cursor = since_cursor.to_string();

        for team_key in teams {
            let (docs, cursor) = self.fetch_team(&client, &team_key, since_cursor).await?;
            if cursor > next_cursor {
                next_cursor = cursor;
            }
            all_docs.extend(docs);
        }
        Ok((all_docs, next_cursor))
    }

    async fn fetch_team(
        &self,
        client: &Client,
        team_key: &str,
        since_cursor: &str,
    ) -> Result<(Vec<IngestionDoc>, String), ConnectorError> {
        let mut docs = Vec::new();
        let mut next_cursor = String::new();
        let mut cursor: Option<String> = None;

        loop {
            let after = cursor.as_deref().unwrap_or("");
            let query = build_issues_query(team_key, after, since_cursor);

            let resp = client
                .post(&self.base_url)
                .header("Authorization", &self.api_key)
                .json(&serde_json::json!({ "query": query }))
                .send()
                .await
                .map_err(|e| ConnectorError(e.to_string()))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                // Linear returns 401 for bad token, 403 for scope
                return Err(ConnectorError(format!("Linear {}: {}", status, &text[..text.len().min(500)])));
            }
            let data: Value = resp.json().await.map_err(|e| ConnectorError(e.to_string()))?;
            // GraphQL errors
            super::check_graphql_errors(&data, "Linear")?;

            let nodes = data
                .pointer("/data/issues/nodes")
                .and_then(|v| v.as_array())
                .or_else(|| data.pointer("/data/team/issues/nodes").and_then(|v| v.as_array()))
                .cloned()
                .unwrap_or_default();

            for node in &nodes {
                parse_issue(node, &mut docs, &mut next_cursor);
            }

            let page_info = data
                .pointer("/data/issues/pageInfo")
                .or_else(|| data.pointer("/data/team/issues/pageInfo"));
            let has_next = page_info.and_then(|v| v.get("hasNextPage")).and_then(|v| v.as_bool()).unwrap_or(false);
            if !has_next {
                break;
            }
            let end_cursor = page_info.and_then(|v| v.get("endCursor")).and_then(|v| v.as_str());
            let Some(ec) = end_cursor else {
                break;
            };
            cursor = Some(ec.to_string());
        }
        Ok((docs, next_cursor))
    }

    pub async fn list_teams(base_url: &str, api_key: &str) -> Result<Vec<(String, String)>, ConnectorError> {
        let url = if base_url.is_empty() { "https://api.linear.app/graphql".to_string() } else { base_url.trim_end_matches('/').to_string() };
        let client = super::http_client(30)?;
        let query = r#"query { teams { nodes { id key name } } }"#;
        let resp = client
            .post(&url)
            .header("Authorization", api_key.trim())
            .json(&serde_json::json!({ "query": query }))
            .send()
            .await
            .map_err(|e| ConnectorError(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(ConnectorError(format!("Linear {}: {}", resp.status(), resp.text().await.unwrap_or_default())));
        }
        let data: Value = resp.json().await.map_err(|e| ConnectorError(e.to_string()))?;
        super::check_graphql_errors(&data, "Linear")?;
        let mut out = Vec::new();
        if let Some(nodes) = data.pointer("/data/teams/nodes").and_then(|v| v.as_array()) {
            for n in nodes {
                if let (Some(k), Some(name)) = (n.get("key").and_then(|v| v.as_str()), n.get("name").and_then(|v| v.as_str())) {
                    out.push((k.to_string(), name.to_string()));
                }
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requires_team_and_key() {
        let cfg = serde_json::json!({"team": "ENG"});
        assert!(LinearConnector::new(&cfg).is_err());
        let cfg2 = serde_json::json!({"api_key": "lin_api_xxx", "team": "ENG"});
        assert!(LinearConnector::new(&cfg2).is_ok());
    }

    #[test]
    fn team_split() {
        let cfg = serde_json::json!({"api_key": "k", "team": "ENG, PM , DESIGN"});
        let c = LinearConnector::new(&cfg).unwrap();
        assert_eq!(c.teams(), vec!["ENG", "PM", "DESIGN"]);
    }

    #[test]
    fn issues_query_has_no_stray_backslashes() {
        // The query template is a raw string: a `\"` there reaches Linear
        // literally and fails parsing (GRAPHQL_PARSE_FAILED at 2:65).
        let q = build_issues_query("ENG", "", "");
        assert!(!q.contains('\\'), "backslash leaked into GraphQL: {q}");
        assert!(q.contains(r#"eq: "ENG""#), "team filter missing: {q}");
        assert!(q.contains("issues(first: 50 filter:"), "shape changed: {q}");
    }

    #[test]
    fn issues_query_paging_and_since() {
        let q = build_issues_query("ENG", "cursor1", "2026-01-02T03:04:05Z");
        assert!(q.contains(r#"after: "cursor1""#), "after clause missing: {q}");
        assert!(q.contains(r#"gt: "2026-01-02T03:04:05Z""#), "since filter missing: {q}");
        assert!(!q.contains('\\'), "backslash leaked into GraphQL: {q}");
    }

    #[test]
    fn issues_query_escapes_team_key() {
        let q = build_issues_query("E\"N\\G", "", "");
        assert!(q.contains(r#"eq: "E\"N\\G""#), "team key not escaped: {q}");
        assert_eq!(q.chars().filter(|&c| c == '\\').count(), 3, "unexpected backslashes: {q}");
    }
}
