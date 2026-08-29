use super::{ConnectorError, IngestionDoc};
use chrono::{DateTime, Utc};
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

fn graphql_error(data: &Value) -> Result<(), ConnectorError> {
    if let Some(errors) = data.get("errors").and_then(|v| v.as_array()) {
        if !errors.is_empty() {
            let msg = errors[0].get("message").and_then(|v| v.as_str()).unwrap_or("graphql error");
            return Err(ConnectorError(format!("Linear GraphQL: {}", msg)));
        }
    }
    Ok(())
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
    let updated = DateTime::parse_from_rfc3339(&updated_raw).ok().map(|d| d.with_timezone(&Utc));
    let mut parts = vec![title.clone(), format!("State: {}", state), format!("Assignee: {}", assignee), desc.clone()];
    if let Some(comments) = node.get("comments").and_then(|v| v.get("nodes")).and_then(|v| v.as_array()) {
        for c in comments {
            if let Some(b) = c.get("body").and_then(|v| v.as_str()) {
                if !b.is_empty() { parts.push(b.to_string()); }
            }
        }
    }
    let text = parts.into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join("\n\n");
    if let Some(dt) = updated {
        let iso = dt.to_rfc3339();
        if next_cursor.is_empty() || iso > *next_cursor { *next_cursor = iso; }
    }
    let author = if !creator.is_empty() { creator } else { assignee.clone() };
    docs.push(IngestionDoc {
        external_id: if identifier.is_empty() { id.clone() } else { identifier.clone() },
        title: if title.is_empty() { identifier.clone() } else { title },
        text,
        author,
        updated_at: updated,
        source_ref: format!("Linear:{}", if identifier.is_empty() { id } else { identifier }),
    });
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
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .map_err(|e| ConnectorError(e.to_string()))?;

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
            // GraphQL: filter by team key and updatedAt > since
            // Use `issues` root query with filter; fallback to `team.issues` if needed.
            let filter_since = if since_cursor.is_empty() {
                "".to_string()
            } else {
                format!(", updatedAt: {{ gt: \"{}\" }}", since_cursor)
            };
            let after = cursor.as_deref().unwrap_or("");
            let after_clause = if after.is_empty() { "".to_string() } else { format!(", after: \"{}\"", after) };
            // Single query that works for both single and multiple teams — filter by team key
            let query = format!(
                r#"query {{
                  issues(first: 50{} filter: {{ team: {{ key: {{ eq: \"{}\" }} }}{} }}) {{
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
                after_clause, team_key, filter_since
            );

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
            graphql_error(&data)?;

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
        let client = Client::builder().timeout(std::time::Duration::from_secs(30)).build().map_err(|e| ConnectorError(e.to_string()))?;
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
        if let Some(errors) = data.get("errors").and_then(|v| v.as_array()) {
            if !errors.is_empty() {
                let msg = errors[0].get("message").and_then(|v| v.as_str()).unwrap_or("graphql error");
                return Err(ConnectorError(format!("Linear GraphQL: {}", msg)));
            }
        }
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
    use tempfile::tempdir;

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
}
