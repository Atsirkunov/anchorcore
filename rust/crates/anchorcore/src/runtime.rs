//! R20.4: `runtime.json` discovery — the running app advertises where it
//! listens so `anchorcore mcp` needs no `ANCHOR_BACKEND_URL` on a custom port.
//!
//! The server writes `{data_dir}/runtime.json` (`port`, `host`, `pid`,
//! `version`) atomically after the listener binds and removes it on graceful
//! shutdown. The MCP side reads it from the same data dir (same resolution as
//! the app: `--data-dir` → `ANCHOR_DATA_DIR` → `ANCHOR_DATABASE_URL` parent →
//! `~/.anchorcore` → `./data`) and only trusts it after a `/health` probe, so
//! a file left behind by a crash never breaks the default-port path.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const FILE_NAME: &str = "runtime.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeInfo {
    pub port: u16,
    pub host: String,
    pub pid: u32,
    pub version: String,
}

impl RuntimeInfo {
    /// This process, listening on `host:port`.
    pub fn current(host: &str, port: u16) -> Self {
        Self {
            port,
            host: host.to_string(),
            pid: std::process::id(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }

    /// Loopback URL a local client should dial. Wildcard binds (`0.0.0.0`,
    /// `::`) are reachable on loopback; IPv6 literals get brackets.
    pub fn base_url(&self) -> String {
        let host = match self.host.trim() {
            "" | "0.0.0.0" => "127.0.0.1".to_string(),
            "::" | "[::]" => "[::1]".to_string(),
            h if h.contains(':') && !h.starts_with('[') => format!("[{h}]"),
            h => h.to_string(),
        };
        format!("http://{}:{}", host, self.port)
    }
}

pub fn path_in(data_dir: &Path) -> PathBuf {
    data_dir.join(FILE_NAME)
}

/// Atomic write: temp file in the same dir, then rename over the target —
/// a reader never sees a half-written file.
pub fn write(data_dir: &Path, info: &RuntimeInfo) -> std::io::Result<()> {
    let body = serde_json::to_vec_pretty(info).map_err(std::io::Error::other)?;
    let tmp = data_dir.join(format!(".{}.{}.tmp", FILE_NAME, info.pid));
    std::fs::write(&tmp, body)?;
    std::fs::rename(&tmp, path_in(data_dir)).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// `None` when absent or unparseable (a corrupt file is the same as none).
pub fn read(data_dir: &Path) -> Option<RuntimeInfo> {
    let raw = std::fs::read(path_in(data_dir)).ok()?;
    serde_json::from_slice(&raw).ok()
}

/// Best-effort removal on graceful shutdown. Only removes a file this pid
/// wrote — a second instance on another port sharing the data dir keeps its
/// own advertisement.
pub fn remove_own(data_dir: &Path, pid: u32) {
    if read(data_dir).is_some_and(|i| i.pid == pid) {
        let _ = std::fs::remove_file(path_in(data_dir));
    }
}

/// Data dir resolution shared by the server and `anchorcore mcp`.
pub fn resolve_data_dir(cli: &str) -> PathBuf {
    if !cli.is_empty() {
        return PathBuf::from(cli);
    }
    if let Some(v) = non_empty_env("ANCHOR_DATA_DIR") {
        return PathBuf::from(v);
    }
    if let Some(parent) = database_url_parent() {
        return parent;
    }
    // Double-click app (no env, no --data-dir): use ~/.anchorcore like Python's run_app.py
    // (was `data` — wrong when launched via Finder, CWD is /). Fallback to `data` for dev checkout.
    if let Ok(home) = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")) {
        let p = PathBuf::from(home).join(".anchorcore");
        // use home dir if it exists or can be created, else fallback to ./data for dev
        if p.exists() || std::fs::create_dir_all(&p).is_ok() {
            return p;
        }
    }
    PathBuf::from("data")
}

fn non_empty_env(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.is_empty())
}

/// `ANCHOR_DATABASE_URL=sqlite:///path/anchorcore.db` → `/path`.
fn database_url_parent() -> Option<PathBuf> {
    let v = std::env::var("ANCHOR_DATABASE_URL").ok()?;
    let p = v.strip_prefix("sqlite:///")?;
    let parent = Path::new(p).parent()?;
    (!parent.as_os_str().is_empty()).then(|| parent.to_path_buf())
}

/// R20.11: args for an MCP client entry. A client launches `anchorcore mcp`
/// without our env, so it lands on `~/.anchorcore`; any other running data
/// dir must be passed explicitly (absolute — the client's cwd is unknown).
pub fn mcp_args(running: &Path) -> Vec<String> {
    let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).ok();
    mcp_args_for(running, home.map(|h| PathBuf::from(h).join(".anchorcore")).as_deref())
}

fn mcp_args_for(running: &Path, home_default: Option<&Path>) -> Vec<String> {
    let abs = absolute(running);
    if home_default.map(absolute).as_ref() == Some(&abs) {
        return vec!["mcp".to_string()];
    }
    vec!["mcp".to_string(), "--data-dir".to_string(), abs.to_string_lossy().to_string()]
}

fn absolute(p: &Path) -> PathBuf {
    if let Ok(c) = p.canonicalize() {
        return c;
    }
    std::env::current_dir().map(|cwd| cwd.join(p)).unwrap_or_else(|_| p.to_path_buf())
}

/// `--data-dir X` / `--data-dir=X` from an argv tail (MCP mode skips clap).
pub fn data_dir_arg<I: IntoIterator<Item = String>>(args: I) -> String {
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        if a == "--data-dir" {
            return it.next().unwrap_or_default();
        }
        if let Some(v) = a.strip_prefix("--data-dir=") {
            return v.to_string();
        }
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_read_round_trip_and_owned_removal() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read(dir.path()).is_none());
        let info = RuntimeInfo::current("127.0.0.1", 8123);
        write(dir.path(), &info).unwrap();
        assert_eq!(read(dir.path()), Some(info.clone()));
        let names: Vec<_> = std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names, vec![std::ffi::OsString::from(FILE_NAME)], "no temp file left behind");
        let raw: serde_json::Value = serde_json::from_slice(&std::fs::read(path_in(dir.path())).unwrap()).unwrap();
        for key in ["port", "host", "pid", "version"] {
            assert!(raw.get(key).is_some(), "{key} missing: {raw}");
        }
        remove_own(dir.path(), info.pid + 1);
        assert!(read(dir.path()).is_some(), "another pid must not remove it");
        remove_own(dir.path(), info.pid);
        assert!(read(dir.path()).is_none());
    }

    #[test]
    fn corrupt_file_reads_as_none() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(path_in(dir.path()), b"{not json").unwrap();
        assert!(read(dir.path()).is_none());
    }

    #[test]
    fn base_url_maps_wildcards_to_loopback() {
        let mut i = RuntimeInfo::current("0.0.0.0", 9000);
        assert_eq!(i.base_url(), "http://127.0.0.1:9000");
        i.host = "::".into();
        assert_eq!(i.base_url(), "http://[::1]:9000");
        i.host = "::1".into();
        assert_eq!(i.base_url(), "http://[::1]:9000");
        i.host = "localhost".into();
        assert_eq!(i.base_url(), "http://localhost:9000");
    }

    #[test]
    fn mcp_args_add_data_dir_only_off_default() {
        let home = tempfile::tempdir().unwrap();
        let default = home.path().join(".anchorcore");
        std::fs::create_dir_all(&default).unwrap();
        assert_eq!(mcp_args_for(&default, Some(&default)), vec!["mcp"]);
        let other = home.path().join("work");
        std::fs::create_dir_all(&other).unwrap();
        let want = other.canonicalize().unwrap().to_string_lossy().to_string();
        assert_eq!(mcp_args_for(&other, Some(&default)), vec!["mcp", "--data-dir", want.as_str()]);
        assert_eq!(mcp_args_for(&other, None)[1], "--data-dir", "no home: always explicit");
        let rel = mcp_args_for(Path::new("rel-data"), Some(&default));
        assert!(Path::new(&rel[2]).is_absolute(), "relative dirs are made absolute: {rel:?}");
    }

    #[test]
    fn data_dir_arg_forms() {
        let v = |a: &[&str]| data_dir_arg(a.iter().map(|s| s.to_string()));
        assert_eq!(v(&["--data-dir", "/x"]), "/x");
        assert_eq!(v(&["--data-dir=/y"]), "/y");
        assert_eq!(v(&["--port", "1"]), "");
        assert_eq!(v(&["--data-dir"]), "");
    }
}
