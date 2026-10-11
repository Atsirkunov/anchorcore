use super::{ConnectorError, IngestionDoc};
use crate::extract;
use chrono::{DateTime, Utc};
use std::path::PathBuf;
use walkdir::WalkDir;

pub struct FolderConnector {
    path: PathBuf,
}

impl FolderConnector {
    pub fn new(config: &serde_json::Value) -> Result<Self, ConnectorError> {
        let p = config
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ConnectorError("folder connector requires 'path'".to_string()))?;
        let expanded = if p.starts_with('~') {
            p.replacen('~', &std::env::var("HOME").unwrap_or_default(), 1)
        } else { p.to_string() };
        let path = PathBuf::from(&expanded);
        let path = path.canonicalize().unwrap_or(path);
        // R9.2: allow non-existent path at creation (config-only, like Jira) — sync will fail with "does not exist" and record error_count, mirroring Python which only validates on sync
        Ok(Self { path })
    }

    pub fn fetch(&self) -> Result<(Vec<IngestionDoc>, String), ConnectorError> {
        let (docs, _skipped, cursor) = self.fetch_with_skipped()?;
        Ok((docs, cursor))
    }

    /// B57 safety: files that exist but could not be read (e.g. a corrupt PDF)
    /// are reported as skipped — purge must never mistake them for deleted.
    pub fn fetch_with_skipped(&self) -> Result<(Vec<IngestionDoc>, Vec<String>, String), ConnectorError> {
        if !self.path.exists() {
            return Err(ConnectorError(format!("folder does not exist: {}", self.path.display())));
        }
        if !self.path.is_dir() {
            return Err(ConnectorError(format!("folder does not exist: {}", self.path.display())));
        }
        let mut docs = Vec::new();
        let mut skipped = Vec::new();
        for entry in WalkDir::new(&self.path).into_iter().filter_map(|e| e.ok()) {
            let p = entry.path();
            if !p.is_file() || !extract::is_supported_path(p) {
                continue;
            }
            let rel = p.strip_prefix(&self.path).unwrap_or(p).to_string_lossy().to_string();
            let text = match extract::extract_file(p) {
                Ok(t) => t,
                Err(e) => {
                    tracing::warn!("folder file skipped ({}): {}", rel, e);
                    skipped.push(rel);
                    continue;
                }
            };
            let mtime = std::fs::metadata(p)
                .ok()
                .and_then(|m| m.modified().ok())
                .map(|t| DateTime::<Utc>::from(t));
            docs.push(IngestionDoc {
                external_id: rel,
                title: p.file_name().unwrap().to_string_lossy().to_string(),
                text,
                author: String::new(),
                updated_at: mtime,
                source_ref: p.to_string_lossy().to_string(),
            });
        }
        Ok((docs, skipped, String::new()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn reads_supported() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.md"), "# Hello\nWorld").unwrap();
        std::fs::write(dir.path().join("b.txt"), "plain").unwrap();
        std::fs::write(dir.path().join("c.pdf"), "not a real pdf").unwrap();
        // business format: routed through the shared extractor (extract.rs)
        std::fs::write(dir.path().join("d.rtf"), r"{\rtf1\ansi Hello \b world\b0.\par}").unwrap();
        let cfg = serde_json::json!({"path": dir.path().to_string_lossy()});
        let c = FolderConnector::new(&cfg).unwrap();
        let (docs, skipped, _) = c.fetch_with_skipped().unwrap();
        assert_eq!(docs.len(), 3);
        assert!(docs.iter().any(|d| d.title=="a.md"));
        assert!(docs.iter().any(|d| d.title=="d.rtf" && d.text.contains("Hello world")));
        // an unreadable file is reported, not silently dropped (purge safety)
        assert_eq!(skipped.len(), 1);
        assert!(skipped[0].ends_with("c.pdf"));
    }
}
