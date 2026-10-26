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

    /// Spawn the watcher thread (runs forever). Call from inside the runtime.
    #[cfg(not(test))]
    pub fn spawn(self: Arc<Self>) -> std::thread::JoinHandle<()> {
        run(self.state.clone())
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

/// B56: cheap per-file fingerprint (mtime + size) so the watcher detects
/// content edits, not just file adds/deletes. Content-hash at sync time
/// remains the authority; this only decides *whether to sync*. The snapshot
/// is built straight from the connector's stat listing — the watcher never
/// reads file content.
type FileSnapshot = HashMap<PathBuf, FileFp>;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct FileFp {
    mtime_ms: u64,
    len: u64,
}

fn snapshot_from_listing(files: &[crate::connectors::folder::ListedFile]) -> FileSnapshot {
    files
        .iter()
        .map(|f| (PathBuf::from(&f.rel), FileFp { mtime_ms: f.mtime_ms, len: f.len }))
        .collect()
}

/// Spawn the watcher thread. Caller keeps the `#[cfg(not(test))]` gate in
/// `main.rs` too.
#[cfg(not(test))]
pub fn spawn(state: AppState) -> std::thread::JoinHandle<()> {
    let svc = WatcherService::new(state);
    svc.spawn()
}

/// R30.3: missed-event fallback period (`ANCHOR_WATCH_FALLBACK_SECS`,
/// default 60, min 5). notify events still sync within the debounce.
const FALLBACK_SECS_DEFAULT: u64 = 60;
const FALLBACK_SECS_MIN: u64 = 5;
/// How often enabled folder sources are re-read (new/removed sources).
const DISCOVER_EVERY: std::time::Duration = std::time::Duration::from_secs(10);
const TICK: std::time::Duration = std::time::Duration::from_secs(1);

fn fallback_interval(raw: Option<&str>) -> std::time::Duration {
    let secs = raw
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(FALLBACK_SECS_DEFAULT)
        .max(FALLBACK_SECS_MIN);
    std::time::Duration::from_secs(secs)
}

/// Enabled folder sources with their stored config (one short read).
fn discover_folder_sources(data_dir: &str) -> Vec<(i64, serde_json::Value)> {
    let db_path = crate::db::resolve_db_path(data_dir);
    let Ok(conn) = crate::db::open_db(&db_path) else { return vec![] };
    let Ok(mut stmt) = conn.prepare("SELECT id, config FROM sources WHERE connector='folder' AND enabled=1") else {
        return vec![];
    };
    let Ok(rows) = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))) else {
        return vec![];
    };
    rows.flatten()
        .filter_map(|(id, cfg)| serde_json::from_str(&cfg).ok().map(|v| (id, v)))
        .collect()
}

/// Stat-only listing of a folder source — never reads file content.
fn list_snapshot(cfg: &serde_json::Value) -> Option<(PathBuf, FileSnapshot)> {
    let fc = crate::connectors::folder::FolderConnector::new(cfg).ok()?;
    let files = fc.list_files().ok()?;
    Some((fc.root().to_path_buf(), snapshot_from_listing(&files)))
}

/// Per-source last-seen snapshots; `compare` says whether a sync is due.
#[derive(Default)]
struct Snapshots {
    configs: HashMap<i64, serde_json::Value>,
    known: HashMap<i64, FileSnapshot>,
}

impl Snapshots {
    /// Record a source's baseline; returns the watch root.
    fn track(&mut self, sid: i64, cfg: &serde_json::Value) -> Option<PathBuf> {
        let (root, snapshot) = list_snapshot(cfg)?;
        self.configs.insert(sid, cfg.clone());
        self.known.insert(sid, snapshot);
        Some(root)
    }

    fn forget(&mut self, sid: i64) {
        self.configs.remove(&sid);
        self.known.remove(&sid);
    }

    /// True (and the baseline moves) when the stat listing changed. With
    /// `require_nonempty`, an empty listing (unmounted drive) never counts.
    fn compare(&mut self, sid: i64, require_nonempty: bool) -> bool {
        let Some((_, fresh)) = self.configs.get(&sid).and_then(list_snapshot) else { return false };
        if require_nonempty && fresh.is_empty() {
            return false;
        }
        if self.known.get(&sid) == Some(&fresh) {
            return false;
        }
        self.known.insert(sid, fresh);
        true
    }
}

#[cfg(not(test))]
struct WatchLoop {
    state: AppState,
    rt: tokio::runtime::Handle,
    watchers: HashMap<i64, crate::connectors::watcher::FolderWatcher>,
    snaps: Snapshots,
}

#[cfg(not(test))]
impl WatchLoop {
    /// Start watchers for new sources, drop ones that were removed/disabled.
    fn reconcile(&mut self) {
        let sources = discover_folder_sources(&self.state.data_dir);
        let live: HashSet<i64> = sources.iter().map(|(id, _)| *id).collect();
        self.watchers.retain(|sid, _| live.contains(sid));
        let stale: Vec<i64> = self.snaps.configs.keys().filter(|s| !live.contains(s)).copied().collect();
        for sid in stale {
            self.snaps.forget(sid);
        }
        for (sid, cfg) in sources {
            if self.watchers.contains_key(&sid) {
                continue;
            }
            self.start_watcher(sid, &cfg);
        }
    }

    fn start_watcher(&mut self, sid: i64, cfg: &serde_json::Value) {
        let Some(root) = self.snaps.track(sid, cfg) else { return };
        match crate::connectors::watcher::FolderWatcher::new(&root) {
            Ok(w) => {
                self.watchers.insert(sid, w);
                tracing::info!("watcher started for source {} at {}", sid, root.display());
            }
            Err(e) => {
                self.snaps.forget(sid);
                tracing::debug!("watcher for source {} at {} not started: {}", sid, root.display(), e);
            }
        }
    }

    /// Sources with pending notify events (debounced 3 s, like Python).
    fn poll_changed(&self) -> Vec<i64> {
        let debounce = std::time::Duration::from_secs(3);
        self.watchers.iter().filter(|(_, w)| !w.poll(debounce).is_empty()).map(|(sid, _)| *sid).collect()
    }

    fn sync_if_changed(&mut self, sid: i64, require_nonempty: bool) -> bool {
        if !self.snaps.compare(sid, require_nonempty) {
            return false;
        }
        self.rt.spawn(trigger_sync(self.state.clone(), sid));
        true
    }

    /// notify-driven syncs; then, every `fallback`, a stat compare of the rest.
    fn run(mut self, fallback: std::time::Duration) {
        let mut last_discover: Option<std::time::Instant> = None;
        let mut last_fallback = std::time::Instant::now();
        loop {
            if last_discover.is_none_or(|t| t.elapsed() >= DISCOVER_EVERY) {
                self.reconcile();
                last_discover = Some(std::time::Instant::now());
            }
            let changed = self.poll_changed();
            for &sid in &changed {
                tracing::info!("watcher detected changes for source {}", sid);
                std::thread::sleep(TICK);
                self.sync_if_changed(sid, false);
            }
            if last_fallback.elapsed() >= fallback {
                self.fallback_pass(&changed);
                last_fallback = std::time::Instant::now();
            }
            std::thread::sleep(TICK);
        }
    }

    /// Missed notify events (FSEvents coalescing): stat-compare the rest.
    fn fallback_pass(&mut self, skip: &[i64]) {
        let sids: Vec<i64> = self.watchers.keys().filter(|s| !skip.contains(s)).copied().collect();
        for sid in sids {
            if self.sync_if_changed(sid, true) {
                tracing::info!("watcher fallback detected changes for source {} (missed notify)", sid);
            }
        }
    }
}

/// R30.3: the loop is blocking (notify channel, stat walks, SQLite), so it
/// owns a plain thread and only hops onto the runtime to start a sync.
#[cfg(not(test))]
fn run(state: AppState) -> std::thread::JoinHandle<()> {
    let fallback = fallback_interval(std::env::var("ANCHOR_WATCH_FALLBACK_SECS").ok().as_deref());
    let looper = WatchLoop {
        state,
        rt: tokio::runtime::Handle::current(),
        watchers: HashMap::new(),
        snaps: Snapshots::default(),
    };
    std::thread::Builder::new()
        .name("watcher".into())
        .spawn(move || looper.run(fallback))
        .expect("spawn watcher thread")
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
            // R30.1: background triggers coalesce silently onto the active job
            Err(crate::jobs::CreateJobError::Conflict(active)) => {
                tracing::debug!("watcher source {}: job {} already active, coalescing", sid, active);
                return;
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn write(dir: &std::path::Path, name: &str, content: &str) {
        std::fs::write(dir.join(name), content).unwrap();
    }

    /// The real watcher path: stat listing → snapshot (never reads content).
    fn list_snapshot(dir: &std::path::Path) -> FileSnapshot {
        let cfg = serde_json::json!({"path": dir.to_string_lossy()});
        let fc = crate::connectors::folder::FolderConnector::new(&cfg).unwrap();
        snapshot_from_listing(&fc.list_files().unwrap())
    }

    #[test]
    fn compare_triggers_only_on_change() {
        // R30.3 DoD: unchanged folder → no trigger; edit/add/remove → trigger.
        let dir = tempdir().unwrap();
        write(dir.path(), "a.md", "hello");
        let cfg = serde_json::json!({"path": dir.path().to_string_lossy()});
        let mut snaps = Snapshots::default();
        assert!(snaps.track(7, &cfg).is_some());
        assert!(!snaps.compare(7, true), "untouched folder must not trigger");
        write(dir.path(), "a.md", "hello world");
        assert!(snaps.compare(7, true), "edit");
        assert!(!snaps.compare(7, true), "baseline moved");
        write(dir.path(), "b.md", "new");
        assert!(snaps.compare(7, true), "add");
        std::fs::remove_file(dir.path().join("a.md")).unwrap();
        assert!(snaps.compare(7, true), "remove");
        std::fs::remove_file(dir.path().join("b.md")).unwrap();
        assert!(!snaps.compare(7, true), "empty listing never counts under require_nonempty");
        assert!(snaps.compare(7, false), "notify path accepts an emptied folder");
        assert!(!snaps.compare(99, false), "untracked source");
    }

    #[test]
    fn fallback_interval_defaults_and_clamps() {
        assert_eq!(fallback_interval(None).as_secs(), 60);
        assert_eq!(fallback_interval(Some("junk")).as_secs(), 60);
        assert_eq!(fallback_interval(Some("1")).as_secs(), 5);
        assert_eq!(fallback_interval(Some(" 300 ")).as_secs(), 300);
    }

    #[test]
    fn snapshot_detects_content_edit() {
        let dir = tempdir().unwrap();
        write(dir.path(), "a.md", "hello");
        let before = list_snapshot(dir.path());
        assert_eq!(before.len(), 1);
        assert_eq!(before[&PathBuf::from("a.md")].len, 5);
        write(dir.path(), "a.md", "hello world, edited");
        let after = list_snapshot(dir.path());
        assert_ne!(before, after, "content edit must change the snapshot");
    }

    #[test]
    fn snapshot_detects_same_length_edit() {
        let dir = tempdir().unwrap();
        write(dir.path(), "a.md", "aaaa");
        let before = list_snapshot(dir.path());
        std::thread::sleep(std::time::Duration::from_millis(10));
        write(dir.path(), "a.md", "bbbb");
        let after = list_snapshot(dir.path());
        assert_ne!(
            before, after,
            "same-length edit must change mtime fingerprint"
        );
    }

    #[test]
    fn snapshot_stable_when_untouched_and_tracks_add_remove() {
        let dir = tempdir().unwrap();
        write(dir.path(), "a.md", "hello");
        let first = list_snapshot(dir.path());
        let second = list_snapshot(dir.path());
        assert_eq!(first, second, "untouched files must not trigger");
        write(dir.path(), "b.md", "new");
        assert_ne!(first, list_snapshot(dir.path()));
        std::fs::remove_file(dir.path().join("a.md")).unwrap();
        let after_rm = list_snapshot(dir.path());
        assert!(!after_rm.contains_key(&PathBuf::from("a.md")));
        assert_ne!(after_rm, first);
    }
}
