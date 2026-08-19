pub mod folder;
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
