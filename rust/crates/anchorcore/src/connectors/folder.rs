use super::{ConnectorError, IngestionDoc};
use chrono::{DateTime, Utc};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

const SUPPORTED: &[&str] = &[".txt", ".md", ".markdown", ".text", ".csv", ".json", ".pdf", ".html", ".htm"];

fn is_supported(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .map(|e| SUPPORTED.contains(&format!(".{}", e.to_lowercase()).as_str()))
        .unwrap_or(false)
}

fn read_plain_text(path: &Path) -> Result<String, ConnectorError> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    match ext.as_str() {
        "txt" | "md" | "markdown" | "text" | "csv" | "json" => {
            std::fs::read_to_string(path)
                .map_err(|e| ConnectorError(format!("read {}: {}", path.display(), e)))
        }
        "html" | "htm" => {
            let s = std::fs::read_to_string(path)
                .map_err(|e| ConnectorError(format!("read {}: {}", path.display(), e)))?;
            // strip tags like Python's re
            let re_script = regex::Regex::new(r"<script[^>]*>.*?</script>").unwrap();
            let re_style = regex::Regex::new(r"<style[^>]*>.*?</style>").unwrap();
            let re_tag = regex::Regex::new(r"<[^>]+>").unwrap();
            let re_ws = regex::Regex::new(r"\s+").unwrap();
            let mut t = re_script.replace_all(&s, " ").to_string();
            t = re_style.replace_all(&t, " ").to_string();
            t = re_tag.replace_all(&t, " ").to_string();
            Ok(re_ws.replace_all(&t, " ").trim().to_string())
        }
        "pdf" => Err(ConnectorError(format!("PDF not yet supported in Rust port: {}", path.display()))),
        _ => Err(ConnectorError(format!("unsupported file type: {:?}", ext))),
    }
}

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
        if !path.is_dir() {
            return Err(ConnectorError(format!("folder does not exist: {}", path.display())));
        }
        Ok(Self { path })
    }

    pub fn fetch(&self) -> Result<(Vec<IngestionDoc>, String), ConnectorError> {
        let mut docs = Vec::new();
        for entry in WalkDir::new(&self.path).into_iter().filter_map(|e| e.ok()) {
            let p = entry.path();
            if !p.is_file() || !is_supported(p) {
                continue;
            }
            let text = match read_plain_text(p) {
                Ok(t) => t,
                Err(_) => continue,
            };
            let mtime = std::fs::metadata(p)
                .ok()
                .and_then(|m| m.modified().ok())
                .map(|t| DateTime::<Utc>::from(t));
            let rel = p.strip_prefix(&self.path).unwrap_or(p).to_string_lossy().to_string();
            docs.push(IngestionDoc {
                external_id: rel,
                title: p.file_name().unwrap().to_string_lossy().to_string(),
                text,
                author: String::new(),
                updated_at: mtime,
                source_ref: p.to_string_lossy().to_string(),
            });
        }
        Ok((docs, String::new()))
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
        std::fs::write(dir.path().join("c.pdf"), "x").unwrap(); // pdf unsupported in Rust stub -> skipped
        let cfg = serde_json::json!({"path": dir.path().to_string_lossy()});
        let c = FolderConnector::new(&cfg).unwrap();
        let (docs, _) = c.fetch().unwrap();
        assert_eq!(docs.len(), 2);
        assert!(docs.iter().any(|d| d.title=="a.md"));
    }
}
