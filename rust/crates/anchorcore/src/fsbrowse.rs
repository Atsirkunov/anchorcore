//! Local folder browsing for the "Browse…" folder picker (Sources add/edit
//! form + onboarding wizard).
//!
//! `GET /system/fs?path=<abs dir>` lists the *child directories* of `path`
//! (never files), so the UI can render a native-dialog-like folder browser.
//! Without `path`, it returns the shortcut view: home + drive roots.
//!
//! Local-first: the Axum process runs on the same machine as the browser
//! (Team: the container), so listing its filesystem is the point. Read-only —
//! directory names only, no file contents. Host/Origin + auth middleware apply
//! like every other `/system` route. Paths are returned as given and never
//! canonicalized (`canonicalize()` would leak the Windows `\\?\` verbatim
//! prefix into source configs — B53).

use axum::{extract::Query, http::StatusCode, Json};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// Bound the listing so a giant directory cannot freeze the picker.
const MAX_DIRS: usize = 500;

#[derive(Deserialize)]
pub struct FsQuery {
    pub path: Option<String>,
}

fn home_dir() -> Option<PathBuf> {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from)
}

#[cfg(windows)]
fn platform_roots() -> Vec<PathBuf> {
    (b'A'..=b'Z')
        .map(|c| PathBuf::from(format!("{}:\\", c as char)))
        .filter(|p| p.is_dir())
        .collect()
}

#[cfg(not(windows))]
fn platform_roots() -> Vec<PathBuf> {
    vec![PathBuf::from("/")]
}

fn entry_json(name: &str, path: &Path) -> Value {
    json!({ "name": name, "path": path.to_string_lossy() })
}

fn name_lower(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

/// Shortcut rows for the no-path view: Home first, then platform roots.
fn shortcut_rows() -> Vec<Value> {
    let mut rows = vec![];
    if let Some(home) = home_dir() {
        rows.push(entry_json("Home", &home));
    }
    for root in platform_roots() {
        rows.push(entry_json(&root.to_string_lossy(), &root));
    }
    rows
}

fn child_dirs(dir: &Path) -> (Vec<Value>, bool) {
    let mut paths: Vec<PathBuf> = match std::fs::read_dir(dir) {
        Ok(rd) => rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect(),
        Err(_) => vec![],
    };
    paths.sort_by(|a, b| name_lower(a).cmp(&name_lower(b)));
    let truncated = paths.len() > MAX_DIRS;
    paths.truncate(MAX_DIRS);
    let rows = paths
        .iter()
        .map(|p| {
            entry_json(
                &p.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default(),
                p,
            )
        })
        .collect();
    (rows, truncated)
}

/// Sync core (unit-testable): browse `path_arg`, or the shortcut view when it
/// is `None`/empty.
pub(crate) fn browse(path_arg: Option<&str>) -> Result<Value, String> {
    let home = home_dir().map(|h| h.to_string_lossy().to_string());
    let requested = path_arg.map(str::trim).filter(|s| !s.is_empty());
    let Some(raw) = requested else {
        return Ok(json!({
            "path": Value::Null,
            "parent": Value::Null,
            "home": home,
            "dirs": shortcut_rows(),
            "truncated": false,
        }));
    };
    let dir = PathBuf::from(raw);
    if !dir.is_absolute() {
        return Err(format!("not an absolute path: {}", raw));
    }
    if !dir.is_dir() {
        return Err(format!("not a folder: {}", raw));
    }
    let (dirs, truncated) = child_dirs(&dir);
    let parent = dir
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(|p| p.to_string_lossy().to_string());
    Ok(json!({
        "path": dir.to_string_lossy(),
        "parent": parent,
        "home": home,
        "dirs": dirs,
        "truncated": truncated,
    }))
}

pub async fn fs_browse_handler(
    Query(q): Query<FsQuery>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let path = q.path;
    let result = tokio::task::spawn_blocking(move || browse(path.as_deref()))
        .await
        .unwrap();
    match result {
        Ok(v) => Ok(Json(v)),
        Err(detail) => Err((StatusCode::BAD_REQUEST, Json(json!({ "detail": detail })))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_only_child_directories_sorted() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("zeta")).unwrap();
        std::fs::create_dir(tmp.path().join("Alpha")).unwrap();
        std::fs::write(tmp.path().join("file.txt"), "x").unwrap();
        let v = browse(Some(tmp.path().to_str().unwrap())).unwrap();
        let names: Vec<&str> = v["dirs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| d["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, vec!["Alpha", "zeta"]);
        assert!(!v["truncated"].as_bool().unwrap());
        assert_eq!(v["path"].as_str().unwrap(), tmp.path().to_str().unwrap());
        // children are returned under the requested path, never canonicalized
        assert!(v["dirs"][0]["path"]
            .as_str()
            .unwrap()
            .starts_with(tmp.path().to_str().unwrap()));
    }

    #[test]
    fn rejects_relative_and_missing_paths() {
        assert!(browse(Some("relative/folder")).is_err());
        let missing = tempfile::tempdir().unwrap().path().join("nope");
        assert!(browse(Some(missing.to_str().unwrap())).is_err());
    }

    #[test]
    fn parent_is_reported_for_nested_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let child = tmp.path().join("child");
        std::fs::create_dir(&child).unwrap();
        let v = browse(Some(child.to_str().unwrap())).unwrap();
        assert_eq!(v["parent"].as_str().unwrap(), tmp.path().to_str().unwrap());
    }

    #[test]
    fn roots_view_lists_shortcuts_without_a_path() {
        let v = browse(None).unwrap();
        assert!(v["path"].is_null());
        assert!(v["parent"].is_null());
        assert!(!v["dirs"].as_array().unwrap().is_empty());
    }
}
