use super::{ConnectorError, IngestionDoc};
use crate::extract;
use chrono::{DateTime, Utc};
use ignore::WalkBuilder;
use std::path::{Path, PathBuf};

/// VCS internals, dependency trees and build output — never worth ingesting
/// even when a repo has no `.gitignore` (matched case-insensitively by name).
/// Known limitation: a user folder that happens to share one of these names
/// (e.g. a notes folder called `build`) is skipped too — only the watched
/// root itself is exempt, so rename such folders to ingest them.
const SKIP_DIRS: &[&str] = &[
    ".git", ".hg", ".svn", "node_modules", "bower_components", "vendor", "__pycache__", ".venv", "venv", "target",
    "dist", "build", ".next", ".nuxt", ".svelte-kit", ".cache", ".parcel-cache", ".tox", ".mypy_cache",
    ".pytest_cache", ".gradle", ".idea", ".vs", ".terraform", "pods", "deriveddata",
];

fn skip_entry(entry: &ignore::DirEntry) -> bool {
    // Never skip the watched root itself, even if it is named like a build dir.
    if entry.depth() == 0 {
        return false;
    }
    entry
        .file_name()
        .to_str()
        .map(|n| SKIP_DIRS.contains(&n.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// Walk a watched folder the way a developer expects for a repo: honor
/// `.gitignore` (even outside a git checkout), keep dotfiles like `.github/`,
/// skip `SKIP_DIRS`, and return only files `extract` can handle — plus image
/// files, which sync stores as asset items (R46.7).
fn supported_files(root: &Path) -> Vec<PathBuf> {
    let walker = WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(true)
        .git_global(false)
        .git_exclude(true)
        .require_git(false)
        .follow_links(false)
        .filter_entry(|e| !skip_entry(e))
        .build();
    walker
        .filter_map(|e| e.ok())
        .map(|e| e.into_path())
        .filter(|p| p.is_file() && (extract::is_supported_path(p) || crate::assets::is_image_path(p)))
        .collect()
}

/// One supported file plus its cheap stat fingerprint — a listing never reads
/// content, so an incremental sync can skip unchanged files without
/// extracting them. Content hash at sync time stays the authority for files
/// whose fingerprint changed; this only decides *whether to extract*.
#[derive(Clone, Debug)]
pub struct ListedFile {
    pub rel: String,
    pub path: PathBuf,
    pub mtime_ms: u64,
    pub len: u64,
}

fn fingerprint_stat(path: &Path) -> Option<(u64, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime_ms = meta
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_millis() as u64;
    Some((mtime_ms, meta.len()))
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
        // R9.2: allow non-existent path at creation (config-only, like Jira) — sync will fail with "does not exist" and record error_count, mirroring Python which only validates on sync
        Ok(Self { path })
    }

    /// The canonical watched root (relative image links resolve inside it).
    pub fn root(&self) -> &Path {
        &self.path
    }

    pub fn fetch(&self) -> Result<(Vec<IngestionDoc>, String), ConnectorError> {
        let (docs, _skipped, cursor) = self.fetch_with_skipped()?;
        Ok((docs, cursor))
    }

    fn ensure_dir(&self) -> Result<(), ConnectorError> {
        if !self.path.exists() || !self.path.is_dir() {
            return Err(ConnectorError(format!("folder does not exist: {}", self.path.display())));
        }
        Ok(())
    }

    /// Cheap listing of every supported file + stat fingerprint — no content
    /// is read. Files that vanish mid-walk are dropped (purge treats a file
    /// missing from the listing as deleted). Sorted by relative path so sync
    /// order is stable across runs.
    pub fn list_files(&self) -> Result<Vec<ListedFile>, ConnectorError> {
        self.ensure_dir()?;
        let mut out = Vec::new();
        for p in supported_files(&self.path) {
            let rel = p.strip_prefix(&self.path).unwrap_or(&p).to_string_lossy().to_string();
            if let Some((mtime_ms, len)) = fingerprint_stat(&p) {
                out.push(ListedFile { rel, path: p, mtime_ms, len });
            }
        }
        out.sort_by(|a, b| a.rel.cmp(&b.rel));
        Ok(out)
    }

    /// Extract one listed file. The caller (incremental sync) decides *which*
    /// files need extraction; unreadable files are the caller's `skipped`.
    /// The note carries the PDF catcher's report for pipeline `system_events`.
    pub fn extract_listed(f: &ListedFile) -> (Result<String, String>, Option<extract::PdfQualityNote>) {
        extract::extract_file_noted(&f.path)
    }

    /// B57 safety: files that exist but could not be read (e.g. a corrupt PDF)
    /// are reported as skipped — purge must never mistake them for deleted.
    pub fn fetch_with_skipped(&self) -> Result<(Vec<IngestionDoc>, Vec<String>, String), ConnectorError> {
        self.ensure_dir()?;
        let mut docs = Vec::new();
        let mut skipped = Vec::new();
        // Raster images have no text; only the incremental sync stores them.
        for f in self.list_files()?.into_iter().filter(|f| !crate::assets::is_raster_image_path(&f.path)) {
            let t_extract = std::time::Instant::now();
            let text = match Self::extract_listed(&f) {
                (Ok(t), _) => t,
                (Err(e), _) => {
                    tracing::warn!("folder file skipped ({}): {}", f.rel, e);
                    skipped.push(f.rel);
                    continue;
                }
            };
            let extract_ms = t_extract.elapsed().as_millis();
            if extract_ms >= 200 {
                tracing::debug!(target: "anchorcore::perf", file = %f.rel, ms = extract_ms, chars = text.len(), "extract");
            }
            let mtime = std::fs::metadata(&f.path)
                .ok()
                .and_then(|m| m.modified().ok())
                .map(|t| DateTime::<Utc>::from(t));
            docs.push(IngestionDoc {
                external_id: f.rel,
                title: f.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
                text,
                author: String::new(),
                updated_at: mtime,
                source_ref: f.path.to_string_lossy().to_string(),
                package: None,
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

    #[test]
    fn list_files_stats_without_extracting() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("b.md"), "# B").unwrap();
        std::fs::write(dir.path().join("a.md"), "# A\nmore").unwrap();
        // corrupt-by-extension: fetch would report it as skipped, but a bare
        // listing must include it (a listing never reads content).
        std::fs::write(dir.path().join("c.pdf"), "not a real pdf").unwrap();
        let cfg = serde_json::json!({"path": dir.path().to_string_lossy()});
        let c = FolderConnector::new(&cfg).unwrap();
        let files = c.list_files().unwrap();
        assert_eq!(files.len(), 3);
        // sorted by rel path for a stable sync order
        let rels: Vec<&str> = files.iter().map(|f| f.rel.as_str()).collect();
        assert_eq!(rels, vec!["a.md", "b.md", "c.pdf"]);
        // fingerprints track stat reality
        let a = files.iter().find(|f| f.rel == "a.md").unwrap();
        let meta = std::fs::metadata(dir.path().join("a.md")).unwrap();
        assert_eq!(a.len, meta.len());
        assert!(a.mtime_ms > 0);
        // an edit changes the fingerprint
        std::thread::sleep(std::time::Duration::from_millis(10));
        std::fs::write(dir.path().join("a.md"), "# A\nmore and more").unwrap();
        let files2 = c.list_files().unwrap();
        let a2 = files2.iter().find(|f| f.rel == "a.md").unwrap();
        assert_ne!((a.mtime_ms, a.len), (a2.mtime_ms, a2.len));
        let b = files2.iter().find(|f| f.rel == "b.md").unwrap();
        let b_before = files.iter().find(|f| f.rel == "b.md").unwrap();
        assert_eq!((b.mtime_ms, b.len), (b_before.mtime_ms, b_before.len));
    }

    #[test]
    fn repo_walk_honors_gitignore_and_skips_vendor() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join(".gitignore"), "ignored/\n*.tmp.md\n").unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src").join("main.rs"), "fn main() {}").unwrap();
        std::fs::create_dir_all(dir.path().join("ignored")).unwrap();
        std::fs::write(dir.path().join("ignored").join("skip.md"), "nope").unwrap();
        std::fs::write(dir.path().join("scratch.tmp.md"), "nope").unwrap();
        std::fs::create_dir_all(dir.path().join("node_modules").join("pkg")).unwrap();
        std::fs::write(dir.path().join("node_modules").join("pkg").join("index.js"), "nope").unwrap();
        std::fs::create_dir_all(dir.path().join(".github").join("workflows")).unwrap();
        std::fs::write(dir.path().join(".github").join("workflows").join("ci.yml"), "name: ci\n").unwrap();
        let cfg = serde_json::json!({"path": dir.path().to_string_lossy()});
        let c = FolderConnector::new(&cfg).unwrap();
        let (docs, _skipped, _) = c.fetch_with_skipped().unwrap();
        let ids: Vec<String> = docs.iter().map(|d| d.external_id.replace('\\', "/")).collect();
        assert!(ids.iter().any(|i| i == "src/main.rs"), "{ids:?}");
        assert!(ids.iter().any(|i| i == ".github/workflows/ci.yml"), "{ids:?}");
        assert!(ids.iter().any(|i| i == ".gitignore"), "{ids:?}");
        assert!(!ids.iter().any(|i| i.contains("ignored")), "{ids:?}");
        assert!(!ids.iter().any(|i| i.ends_with("scratch.tmp.md")), "{ids:?}");
        assert!(!ids.iter().any(|i| i.contains("node_modules")), "{ids:?}");
    }
}
