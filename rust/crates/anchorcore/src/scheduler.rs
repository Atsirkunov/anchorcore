//! Scheduler (R4.2) — poll loops `jira_poll_minutes` / `folder_scan_minutes`, `reload_sources` on CRUD.
//! Port of `backend/app/scheduler.py`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::embedder::pack_single;
use crate::jobs::JobManager;
use crate::pipeline::run_db;

use std::sync::OnceLock;
static GLOBAL_SCHEDULER: OnceLock<Arc<Scheduler>> = OnceLock::new();
pub fn set_global(s: Arc<Scheduler>) {
    let _ = GLOBAL_SCHEDULER.set(s);
}
pub fn global() -> Option<Arc<Scheduler>> {
    GLOBAL_SCHEDULER.get().cloned()
}

#[derive(Debug, Clone)]
pub struct SourceSchedule {
    pub source_id: i64,
    pub connector: String,
    pub enabled: bool,
    pub poll_minutes: i64,
    pub last_synced_at: Option<String>,
}

/// R28.2: embedding backfill — port of Python `scheduler.py:_backfill_loop`
/// plus its startup one-shot. Batch size bounds memory on large corpora;
/// 15 minutes matches the Python design. Without this, chunks whose embed
/// failed (Ollama down, model pulling, cancelled job) and IDF-gated chunks
/// stayed NULL forever: `pending_embeddings` never drained and nothing was
/// logged.
const BACKFILL_BATCH: usize = 64;
const BACKFILL_INTERVAL_SECS: u64 = 15 * 60;
const BACKFILL_RETRY_SECS: u64 = 60;
const BACKFILL_RETRIES: u32 = 5;

pub struct Scheduler {
    jobs: Arc<JobManager>,
    handles: Mutex<HashMap<i64, tokio::task::JoinHandle<()>>>,
    data_dir: String,
    settings: Arc<crate::settings::SettingsService>,
    // R11.1: keep AppState pieces to actually run Pipeline (was dead code — only created jobs)
    embedder: Arc<crate::embedder::Embedder>,
}

impl Scheduler {
    pub fn new(jobs: Arc<JobManager>, data_dir: String, settings: Arc<crate::settings::SettingsService>, embedder: Arc<crate::embedder::Embedder>) -> Arc<Self> {
        Arc::new(Self { jobs, handles: Mutex::new(HashMap::new()), data_dir, settings, embedder })
    }
    // legacy new for tests (uses temp dir + empty settings + dummy embedder)
    pub fn new_for_test(jobs: Arc<JobManager>) -> Arc<Self> {
        let dir = std::env::temp_dir().to_string_lossy().to_string();
        let store = crate::secrets::SecretStore::new(std::path::PathBuf::from(&dir).join("secrets.enc"));
        let settings = Arc::new(crate::settings::SettingsService::new(store));
        let embedder = Arc::new(crate::embedder::Embedder::new(settings.clone()));
        Arc::new(Self { jobs, handles: Mutex::new(HashMap::new()), data_dir: dir, settings, embedder })
    }
    pub fn new_with_state(state: crate::health::AppState) -> Arc<Self> {
        Arc::new(Self {
            jobs: state.jobs.clone(),
            handles: Mutex::new(HashMap::new()),
            data_dir: state.data_dir.clone(),
            settings: state.settings.clone(),
            embedder: state.embedder.clone(),
        })
    }

    /// Load all enabled sources and (re)spawn poll tasks. Call on startup and on CRUD.
    pub fn reload_sources(&self, conn: &rusqlite::Connection) {
        tracing::info!("scheduler reload_sources called");
        // Cancel old tasks
        let mut h = self.handles.lock().unwrap();
        for (_, handle) in h.drain() {
            handle.abort();
        }
        // Read enabled sources
        let (jira_poll, folder_poll) = self.poll_intervals_minutes(&self.settings);
        let mut stmt = match conn.prepare("SELECT id, connector, enabled FROM sources WHERE enabled = 1") {
            Ok(s) => s,
            Err(_) => return,
        };
        let rows: Vec<(i64, String, i64)> = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap().filter_map(|r| r.ok()).collect();
        drop(h);
        for (id, connector, _enabled) in rows {
            let minutes = match connector.as_str() {
                "jira" => jira_poll,
                "folder" => folder_poll,
                _ => continue,
            };
            if minutes <= 0 {
                continue;
            }
            let schedule = SourceSchedule {
                source_id: id,
                connector,
                enabled: true,
                poll_minutes: minutes,
                last_synced_at: None,
            };
            self.spawn_for_source(schedule);
        }
    }

    pub fn poll_intervals_minutes(&self, settings: &crate::settings::SettingsService) -> (i64, i64) {
        let jira: i64 = settings.get("jira_poll_minutes", None).and_then(|v| v.parse().ok()).unwrap_or(15);
        let folder: i64 = settings.get("folder_scan_minutes", None).and_then(|v| v.parse().ok()).unwrap_or(60);
        (jira, folder)
    }

    /// Spawn a periodic job creator for one source (used by reload). R11.1: actually creates sync jobs AND runs Pipeline (was stranded running).
    pub fn spawn_for_source(&self, source: SourceSchedule) {
        if !source.enabled || source.poll_minutes <= 0 {
            return;
        }
        let jobs = self.jobs.clone();
        let data_dir = self.data_dir.clone();
        let settings = self.settings.clone();
        let embedder = self.embedder.clone();
        let sid = source.source_id;
        let handle = tokio::spawn(async move {
            let secs = if source.poll_minutes <= 0 { 60 } else { source.poll_minutes as u64 * 60 };
            let mut interval = tokio::time::interval(Duration::from_secs(secs));
            interval.tick().await;
            loop {
                interval.tick().await;
                tracing::info!("scheduler tick source {} ({})", sid, source.connector);
                let db_path = crate::db::resolve_db_path(&data_dir);
                let conn = match crate::db::init_db(&db_path) {
                    Ok(c) => c,
                    Err(_) => continue,
                };
                let job_id = match jobs.create_job(&conn, sid, "sync") {
                    Ok(id) => id,
                    Err(_) => continue,
                };
                // R11.1: mirror watcher/service.rs:202 trigger_sync — actually execute the pipeline
                let data_dir2 = data_dir.clone();
                let settings2 = settings.clone();
                let embedder2 = embedder.clone();
                let jobs2 = jobs.clone();
                tokio::spawn(Box::pin(async move {
                    let classifier = std::sync::Arc::new(crate::classifier::Classifier::new(settings2.clone()));
                    let pipeline = crate::pipeline::Pipeline::new(classifier, embedder2, settings2, data_dir2, jobs2);
                    Box::pin(pipeline.sync_source(sid, job_id, false)).await;
                }));
            }
        });
        self.handles.lock().unwrap().insert(source.source_id, handle);
    }

    /// Spawn the backfill task: one run at startup, then every 15 minutes.
    /// A failed pass retries at 60 s (Ollama often starts after us) instead
    /// of leaving chunks pending for the full interval.
    pub fn spawn_backfill(self: &Arc<Self>) {
        let sched = self.clone();
        tokio::spawn(async move {
            sched.backfill_with_retry().await;
            let mut tick = tokio::time::interval(Duration::from_secs(BACKFILL_INTERVAL_SECS));
            tick.tick().await; // consume the immediate tick
            loop {
                tick.tick().await;
                sched.backfill_with_retry().await;
            }
        });
    }

    /// One backfill pass, retried up to `BACKFILL_RETRIES` times at 60 s
    /// while the embedder fails — covers the Ollama-starts-after-us race
    /// without spamming a dead endpoint forever.
    async fn backfill_with_retry(&self) {
        let mut attempt = 0u32;
        loop {
            let (_, failed) = self.backfill_once().await;
            if !failed || attempt >= BACKFILL_RETRIES {
                return;
            }
            attempt += 1;
            tokio::time::sleep(Duration::from_secs(BACKFILL_RETRY_SECS)).await;
        }
    }

    /// Embed NULL-vector chunks in ascending-id batches until none remain.
    /// Deliberately ungated (no IDF gate): this is the safety net that
    /// drains `pending_embeddings`, and ingest already applied the trust
    /// gates. A failed batch logs + records a system_event and stops; the
    /// caller retries (see `backfill_with_retry`). Returns
    /// `(embedded, failed)`.
    pub async fn backfill_once(&self) -> (usize, bool) {
        let mut embedded = 0usize;
        let mut failed = false;
        loop {
            match self.backfill_batch().await {
                Ok((n, more)) => {
                    embedded += n;
                    if !more {
                        break;
                    }
                }
                Err(e) => {
                    failed = true;
                    let pending = pending_embedding_count(&self.data_dir);
                    tracing::warn!("embedding backfill failed ({}); {} chunks still pending", e, pending);
                    record_backfill_event(&self.data_dir, pending, &e);
                    break;
                }
            }
        }
        if embedded > 0 {
            tracing::info!("embedding backfill: embedded {} chunks", embedded);
        }
        (embedded, failed)
    }

    async fn backfill_batch(&self) -> Result<(usize, bool), String> {
        let data_dir = self.data_dir.clone();
        let batch: Vec<(i64, String)> = run_db(&data_dir, |conn| {
            let mut stmt = conn
                .prepare("SELECT id, content FROM chunks WHERE embedding IS NULL ORDER BY id LIMIT ?1")
                .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map([BACKFILL_BATCH as i64], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))
                .map_err(|e| e.to_string())?;
            Ok(rows.filter_map(|r| r.ok()).collect())
        })
        .await?;
        if batch.is_empty() {
            return Ok((0, false));
        }
        let has_more = batch.len() == BACKFILL_BATCH;
        let texts: Vec<String> = batch.iter().map(|(_, c)| c.clone()).collect();
        let vectors = self.embedder.embed(texts).await?;
        if vectors.len() != batch.len() {
            return Err(format!("embedder returned {} vectors for {} chunks", vectors.len(), batch.len()));
        }
        let updates: Vec<(i64, Vec<u8>)> = batch.iter().map(|(id, _)| *id).zip(vectors).map(|(id, v)| (id, pack_single(&v))).collect();
        let embedded = updates.len();
        let embedder = self.embedder.clone();
        run_db(&data_dir, move |conn| {
            for (cid, blob) in &updates {
                conn.execute("UPDATE chunks SET embedding=?1 WHERE id=?2", rusqlite::params![blob, cid]).map_err(|e| e.to_string())?;
                let _ = embedder.sync_vec(conn, *cid, blob);
            }
            Ok(())
        })
        .await?;
        Ok((embedded, has_more))
    }
}

fn pending_embedding_count(data_dir: &str) -> i64 {
    let conn = crate::common::open_data_db(data_dir);
    conn.query_row("SELECT COUNT(*) FROM chunks WHERE embedding IS NULL", [], |r| r.get(0))
        .unwrap_or(0)
}

fn record_backfill_event(data_dir: &str, pending: i64, err: &str) {
    let detail: String = err.chars().take(500).collect();
    let msg = format!("embedding backfill failed; {} chunks still pending", pending);
    let conn = crate::common::open_data_db(data_dir);
    let _ = conn.execute(
        "INSERT INTO system_events (component, level, message, detail) VALUES ('embedder','warning',?1,?2)",
        rusqlite::params![msg, detail],
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::JobManager;
    use tempfile::tempdir;
    #[test]
    fn reload_idempotent() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.db");
        let conn = crate::db::init_db(&path).unwrap();
        let jobs = JobManager::new();
        let sched = Scheduler::new_for_test(jobs);
        sched.reload_sources(&conn);
        sched.reload_sources(&conn); // second call cancels first
    }
    fn seed_one_item_with_chunks(dir: &tempfile::TempDir, contents: &[&str]) -> (rusqlite::Connection, i64) {
        let db_path = crate::db::resolve_db_path(&dir.path().to_string_lossy().to_string());
        let conn = crate::db::init_db(&db_path).unwrap();
        conn.execute("INSERT INTO sources (name, connector, config, enabled, last_sync_cursor, error_count) VALUES ('s','folder','{}',1,'',0)", []).unwrap();
        let sid = conn.last_insert_rowid();
        conn.execute("INSERT INTO ingested_items (source_id, external_id, title, text, content_hash, author, stale) VALUES (?1,'a.md','a','text','h','',0)", [sid]).unwrap();
        let iid = conn.last_insert_rowid();
        for c in contents {
            conn.execute("INSERT INTO chunks (item_id, kind, source_ref, content) VALUES (?1,'document','r',?2)", rusqlite::params![iid, c]).unwrap();
        }
        (conn, sid)
    }

    /// Minimal OpenAI-compatible embeddings server for one request.
    async fn mock_embed_server() -> (u16, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        use std::sync::atomic::AtomicUsize;
        let hits = std::sync::Arc::new(AtomicUsize::new(0));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let hits_task = hits.clone();
        tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            if let Ok((mut sock, _)) = listener.accept().await {
                let mut buf = vec![0u8; 8192];
                let n = sock.read(&mut buf).await.unwrap_or(0);
                let text = String::from_utf8_lossy(&buf[..n]).to_string();
                let count = text
                    .split("\r\n\r\n")
                    .nth(1)
                    .and_then(|b| serde_json::from_str::<serde_json::Value>(b).ok())
                    .and_then(|v| v.get("input").and_then(|i| i.as_array()).map(|a| a.len()))
                    .unwrap_or(0);
                let data: Vec<serde_json::Value> = (0..count)
                    .map(|i| serde_json::json!({"index": i, "embedding": [0.5f32, 0.25f32]}))
                    .collect();
                let body = serde_json::json!({"data": data}).to_string();
                let head = format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n", body.len());
                let _ = sock.write_all(head.as_bytes()).await;
                let _ = sock.write_all(body.as_bytes()).await;
                hits_task.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
        });
        (port, hits)
    }

    #[tokio::test]
    async fn backfill_embeds_pending_chunks() {
        let dir = tempdir().unwrap();
        // one low-signal chunk ("The") that the ingest IDF gate skips + one normal chunk
        let (conn, _sid) = seed_one_item_with_chunks(&dir, &["The", "billing migration runbook details"]);
        let (port, hits) = mock_embed_server().await;
        let store = crate::secrets::SecretStore::new(dir.path().join("secrets.enc"));
        let svc = std::sync::Arc::new(crate::settings::SettingsService::new(store));
        svc.set_data_dir(dir.path()); // pin the temp DB: cached_get must not fall back to a real local Ollama
        svc.set("embed_base_url", &format!("http://127.0.0.1:{}", port), &conn).unwrap();
        let embedder = std::sync::Arc::new(crate::embedder::Embedder::new(svc.clone()));
        let sched = Scheduler::new(JobManager::new(), dir.path().to_string_lossy().to_string(), svc, embedder);
        let (embedded, failed) = sched.backfill_once().await;
        assert!(!failed);
        assert_eq!(embedded, 2, "both NULL chunks incl. the low-signal one must embed");
        assert!(hits.load(std::sync::atomic::Ordering::SeqCst) >= 1, "the mock embedder must have served the request");
        let pending: i64 = conn.query_row("SELECT COUNT(*) FROM chunks WHERE embedding IS NULL", [], |r| r.get(0)).unwrap();
        assert_eq!(pending, 0, "pending_embeddings must drain");
    }

    #[tokio::test]
    async fn backfill_failure_records_event() {
        let dir = tempdir().unwrap();
        let (conn, _sid) = seed_one_item_with_chunks(&dir, &["orphan chunk"]);
        let store = crate::secrets::SecretStore::new(dir.path().join("secrets.enc"));
        let svc = std::sync::Arc::new(crate::settings::SettingsService::new(store));
        svc.set_data_dir(dir.path()); // pin the temp DB: cached_get must not fall back to a real local Ollama
        // closed port: errors instantly, no Ollama needed
        svc.set("embed_base_url", "http://127.0.0.1:9", &conn).unwrap();
        let embedder = std::sync::Arc::new(crate::embedder::Embedder::new(svc.clone()));
        let sched = Scheduler::new(JobManager::new(), dir.path().to_string_lossy().to_string(), svc, embedder);
        let (embedded, failed) = sched.backfill_once().await;
        assert_eq!(embedded, 0);
        assert!(failed, "a dead embedder must report failure so the caller retries");
        let msg: String = conn
            .query_row("SELECT message FROM system_events WHERE component='embedder' ORDER BY id DESC LIMIT 1", [], |r| r.get(0))
            .unwrap();
        assert!(msg.contains("backfill failed"), "event: {}", msg);
        let pending: i64 = conn.query_row("SELECT COUNT(*) FROM chunks WHERE embedding IS NULL", [], |r| r.get(0)).unwrap();
        assert_eq!(pending, 1, "a failed backfill must leave the chunk pending");
    }

    #[tokio::test]
    async fn backfill_with_retry_drains_on_success_without_sleeping() {
        let dir = tempdir().unwrap();
        let (conn, _sid) = seed_one_item_with_chunks(&dir, &["retry me"]);
        let (port, _hits) = mock_embed_server().await;
        let store = crate::secrets::SecretStore::new(dir.path().join("secrets.enc"));
        let svc = std::sync::Arc::new(crate::settings::SettingsService::new(store));
        svc.set_data_dir(dir.path());
        svc.set("embed_base_url", &format!("http://127.0.0.1:{}", port), &conn).unwrap();
        let embedder = std::sync::Arc::new(crate::embedder::Embedder::new(svc.clone()));
        let sched = Scheduler::new(JobManager::new(), dir.path().to_string_lossy().to_string(), svc, embedder);
        // Success returns immediately (no 60 s sleeps), draining the queue.
        sched.backfill_with_retry().await;
        let pending: i64 = conn.query_row("SELECT COUNT(*) FROM chunks WHERE embedding IS NULL", [], |r| r.get(0)).unwrap();
        assert_eq!(pending, 0);
    }

    #[test]
    fn intervals_default() {
        let dir = tempdir().unwrap();
        let store = crate::secrets::SecretStore::new(dir.path().join("secrets.enc"));
        let svc = crate::settings::SettingsService::new(store);
        let svc_arc = std::sync::Arc::new(svc);
        let embedder = std::sync::Arc::new(crate::embedder::Embedder::new(svc_arc.clone()));
        let jobs = JobManager::new();
        let sched = Scheduler::new(jobs, dir.path().to_string_lossy().to_string(), svc_arc.clone(), embedder);
        let (jira, folder) = sched.poll_intervals_minutes(&svc_arc);
        assert_eq!(jira, 15);
        assert_eq!(folder, 60);
    }
}
