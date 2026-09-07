pub mod folder;
pub mod gdrive;
pub mod jira;
pub mod linear;
pub mod watcher;

use chrono::{DateTime, Utc};

#[derive(Debug, Clone)]
pub struct IngestionDoc {
    pub external_id: String,
    pub title: String,
    pub text: String,
    pub author: String,
    pub updated_at: Option<DateTime<Utc>>,
    pub source_ref: String,
}

#[derive(Debug)]
pub struct ConnectorError(pub String);
impl std::fmt::Display for ConnectorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "{}", self.0) }
}
impl std::error::Error for ConnectorError {}

// --- R15.5: shared HTTP + doc-building helpers (Jira/Linear/GDrive) ---

/// Single `reqwest` constructor — every connector used 60s/30s builder copies.
pub fn http_client(timeout_secs: u64) -> Result<reqwest::Client, ConnectorError> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .build()
        .map_err(|e| ConnectorError(e.to_string()))
}

/// Shared GraphQL `errors` gate (was `graphql_error` in linear.rs + an inline
/// copy in `list_teams`).
pub fn check_graphql_errors(data: &serde_json::Value, prefix: &str) -> Result<(), ConnectorError> {
    if let Some(errors) = data.get("errors").and_then(|v| v.as_array()) {
        if !errors.is_empty() {
            let msg = errors[0].get("message").and_then(|v| v.as_str()).unwrap_or("graphql error");
            return Err(ConnectorError(format!("{} GraphQL: {}", prefix, msg)));
        }
    }
    Ok(())
}

/// Parse an RFC3339 timestamp (Jira `updated`, Linear `updatedAt`, Drive `modifiedTime`).
pub fn parse_dt(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw).ok().map(|d| d.with_timezone(&Utc))
}

/// Track the max `updated` timestamp as the incremental cursor.
pub fn bump_cursor(next_cursor: &mut String, updated: Option<DateTime<Utc>>) {
    if let Some(dt) = updated {
        let iso = dt.to_rfc3339();
        if next_cursor.is_empty() || iso > *next_cursor {
            *next_cursor = iso;
        }
    }
}

/// Join title/body/comment parts, dropping empties (was per-connector `parts` code).
pub fn join_parts(parts: Vec<String>) -> String {
    parts.into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join("\n\n")
}

/// Shared `IngestionDoc` constructor: empty title falls back to the external id,
/// text is the non-empty join, cursor tracks max `updated`.
/// `source_prefix` is `Jira` / `Linear` / `gdrive` (keeps existing `source_ref` shape).
#[allow(clippy::too_many_arguments)]
pub fn push_doc(
    docs: &mut Vec<IngestionDoc>,
    next_cursor: &mut String,
    external_id: &str,
    title: &str,
    parts: Vec<String>,
    author: String,
    updated: Option<DateTime<Utc>>,
    source_prefix: &str,
) {
    let text = join_parts(parts);
    bump_cursor(next_cursor, updated);
    docs.push(IngestionDoc {
        external_id: external_id.to_string(),
        title: if title.is_empty() { external_id.to_string() } else { title.to_string() },
        text,
        author,
        updated_at: updated,
        source_ref: format!("{}:{}", source_prefix, external_id),
    });
}

/// Jira-style page advance: `isLast` ends, else the next token (None ends too).
pub fn next_jql_token(data: &serde_json::Value, token: &mut Option<String>) -> bool {
    if data.get("isLast").and_then(|v| v.as_bool()).unwrap_or(true) {
        return true;
    }
    *token = data.get("nextPageToken").and_then(|v| v.as_str()).map(|s| s.to_string());
    token.is_none()
}
