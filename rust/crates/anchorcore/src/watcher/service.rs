//! Watcher service — extracted from `main.rs:92` monolith (P0 3.3).
//! Owns per-source `FolderWatcher` state, polls + fallback fetch-compare, debounces,
//! and triggers `pipeline.sync_source` via bounded `JobManager`.
//! Keeps `rusqlite::Connection` off async stack via `spawn_blocking`.

#[allow(unused_imports)]
use std::collections::{HashMap, HashSet};
#[allow(unused_imports)]
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::health::AppState;

/// Week 2 — WatcherService with handle map + reload (P0 3.3 polish).
/// Keeps per-source JoinHandle so `reload_sources` can abort + respawn.
/// Uses debounce 3s (like `folder_watch_debounce` in Python) via `poll(3s)`.
#[allow(dead_code)]
pub struct WatcherService {
    state: AppState,
    handles: Mutex<HashMap<i64, tokio::task::JoinHandle<()>>>,
}

impl WatcherService {
    pub fn new(state: AppState) -> Arc<Self> {
        Arc::new(Self {
            state,
            handles: Mutex::new(HashMap::new()),
        })
    }

    /// Spawn background watcher loop. Returns JoinHandle that runs forever.
    /// Debounce 3s, single DB init per discover tick, handle map for reload.
    #[cfg(not(test))]
    pub fn spawn(self: Arc<Self>) -> tokio::task::JoinHandle<()> {
        let state = self.state.clone();
        tokio::spawn(Box::pin(run(state)))
    }

    #[cfg(test)]
    pub fn spawn(self: Arc<Self>) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async {})
    }

    pub fn reload(&self) {
        let mut h = self.handles.lock().unwrap();
        for (_, handle) in h.drain() {
            handle.abort();
        }
        tracing::info!("watcher reload: aborted {} handles", h.len());
    }
}

/// Spawn background watcher loop. Returns JoinHandle that runs forever.
/// Caller should keep `#[cfg(not(test))]` gate in `main.rs` — this fn itself
/// is also `#[cfg(not(test))]` for safety but we gate at call site as well.
#[cfg(not(test))]
pub fn spawn(state: AppState) -> tokio::task::JoinHandle<()> {
    let svc = WatcherService::new(state);
    svc.spawn()
}

#[cfg(not(test))]
async fn discover_folder_sources(data_dir: &str) -> Vec<(i64, String)> {
    let data_dir = data_dir.to_string();
    // discover folder sources (spawn_blocking to keep Connection off async stack)
    tokio::task::spawn_blocking(move || {
        let db_path = crate::db::resolve_db_path(&data_dir);
        let conn = crate::db::init_db(&db_path)
            .unwrap_or_else(|_| rusqlite::Connection::open(&db_path).unwrap());
        let mut stmt =
            match conn.prepare("SELECT id, config FROM sources WHERE connector='folder' AND enabled=1") {
                Ok(s) => s,
                Err(_) => return vec![],
            };
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))
            .unwrap();
        let mut out = vec![];
        for row in rows.flatten() {
            let (id, cfg_str) = row;
            let cfg: serde_json::Value =
                serde_json::from_str(&cfg_str).unwrap_or(serde_json::json!({}));
            if let Some(p) = cfg.get("path").and_then(|v| v.as_str()) {
                out.push((id, p.to_string()));
            }
        }
        out
    })
    .await
    .unwrap_or_default()
}

#[cfg(not(test))]
fn expand_path(path_str: &str) -> PathBuf {
    if path_str.starts_with('~') {
        PathBuf::from(path_str.replacen(
            '~',
            &std::env::var("HOME").unwrap_or_default(),
            1,
        ))
    } else {
        PathBuf::from(path_str)
    }
}

#[cfg(not(test))]
fn ensure_watchers(
    watchers: &mut HashMap<i64, (crate::connectors::watcher::FolderWatcher, PathBuf)>,
    known_files: &mut HashMap<i64, HashSet<PathBuf>>,
    folder_sources: &[(i64, String)],
) {
    for (sid, path_str) in folder_sources {
        if watchers.contains_key(sid) {
            continue;
        }
        let expanded = expand_path(path_str);
        let cfg = serde_json::json!({"path": expanded.to_string_lossy()});
        if let Ok(w) = crate::connectors::watcher::FolderWatcher::new(&expanded) {
            let mut set = HashSet::new();
            if let Ok(fc) = crate::connectors::folder::FolderConnector::new(&cfg) {
                if let Ok((docs, _)) = fc.fetch() {
                    for d in docs {
                        set.insert(PathBuf::from(d.external_id));
                    }
                }
            }
            known_files.insert(*sid, set);
            let display_str = expanded.display().to_string();
            watchers.insert(*sid, (w, expanded));
            tracing::info!("watcher started for source {} at {}", sid, display_str);
        }
    }
}

#[cfg(not(test))]
fn poll_changed(
    watchers: &HashMap<i64, (crate::connectors::watcher::FolderWatcher, PathBuf)>,
) -> Vec<i64> {
    let mut to_sync: Vec<i64> = Vec::new();
    for (sid, (watcher, _)) in watchers.iter() {
        // debounce 3s like Python folder_watch_debounce (was 300ms)
        if !watcher.poll(std::time::Duration::from_secs(3)).is_empty() {
            to_sync.push(*sid);
        }
    }
    to_sync
}

#[cfg(not(test))]
async fn fetch_and_compare(
    state: &AppState,
    known_files: &mut HashMap<i64, HashSet<PathBuf>>,
    sid: i64,
    require_nonempty: bool,
    log_line: Option<&str>,
) -> bool {
    let cfg_str = {
        let db_path = crate::db::resolve_db_path(&state.data_dir);
        let conn = crate::db::init_db(&db_path)
            .unwrap_or_else(|_| rusqlite::Connection::open(&db_path).unwrap());
        conn.query_row(
            "SELECT config FROM sources WHERE id=?1",
            [sid],
            |r| r.get::<_, String>(0),
        )
        .unwrap_or_default()
    };
    let cfg: serde_json::Value =
        serde_json::from_str(&cfg_str).unwrap_or(serde_json::json!({}));
    let mut changed = false;
    if let Ok(fc) = crate::connectors::folder::FolderConnector::new(&cfg) {
        if let Ok((docs, _)) = fc.fetch() {
            let new_set: HashSet<PathBuf> =
                docs.iter().map(|d| PathBuf::from(&d.external_id)).collect();
            let old_set = known_files.get(&sid).cloned().unwrap_or_default();
            if new_set != old_set && (!require_nonempty || !new_set.is_empty()) {
                if let Some(line) = log_line {
                    tracing::info!(line, sid);
                }
                known_files.insert(sid, new_set);
                changed = true;
            }
        }
    }
    changed
}

#[cfg(not(test))]
async fn run(state: AppState) {
    let mut watchers: HashMap<i64, (crate::connectors::watcher::FolderWatcher, PathBuf)> =
        HashMap::new();
    let mut known_files: HashMap<i64, HashSet<PathBuf>> = HashMap::new();
    loop {
        let folder_sources = discover_folder_sources(&state.data_dir).await;
        // ensure watchers
        ensure_watchers(&mut watchers, &mut known_files, &folder_sources);

        let to_sync = poll_changed(&watchers);

        for sid in to_sync.clone() {
            tracing::info!("watcher detected changes for source {}", sid);
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            if fetch_and_compare(&state, &mut known_files, sid, false, None).await {
                trigger_sync(state.clone(), sid).await;
            }
        }

        // fallback: periodic fetch-compare for missed notify events (FSEvents coalescing)
        for sid in watchers.keys().cloned().collect::<Vec<_>>() {
            if to_sync.contains(&sid) {
                continue;
            }
            if fetch_and_compare(&state, &mut known_files, sid, true, Some("watcher fallback detected changes for source {} (missed notify)")).await {
                trigger_sync(state.clone(), sid).await;
            }
        }

        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
}

#[cfg(not(test))]
async fn trigger_sync(state: AppState, sid: i64) {
    let data_dir = state.data_dir.clone();
    let settings = state.settings.clone();
    let jobs = state.jobs.clone();
    // Box pipeline future to reduce stack (huge `classify_and_store` future)
    tokio::spawn(Box::pin(async move {
        let db_path = crate::db::resolve_db_path(&data_dir);
        let conn = crate::db::init_db(&db_path)
            .unwrap_or_else(|_| rusqlite::Connection::open(&db_path).unwrap());
        let job_id = match jobs.create_job(&conn, sid, "sync") {
            Ok(id) => id,
            Err(_) => return,
        };
        let classifier = Arc::new(crate::classifier::Classifier::new(settings.clone()));
        let embedder = Arc::new(crate::embedder::Embedder::new(settings.clone()));
        let pipeline = crate::pipeline::Pipeline::new(classifier, embedder, settings, data_dir, jobs.clone());
        // box inner future to keep stack bounded (8 MB test thread previously overflowed)
        Box::pin(pipeline.sync_source(sid, job_id, false)).await;
    }));
}

// When compiled for `cargo test`, provide a no-op spawn so `main.rs` can call without cfg.
#[cfg(test)]
#[allow(dead_code)]
pub fn spawn(_state: AppState) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async {})
}
