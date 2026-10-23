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
                    // R30.1: background ticks coalesce silently onto the active job
                    Err(crate::jobs::CreateJobError::Conflict(active)) => {
                        tracing::info!("scheduler tick source {}: job {} already active, skipping", sid, active);
                        continue;
                    }
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
    /// of leaving chunks pending for the full interval. R45.5: a query
    /// vector from a new embedding family wakes it immediately — that pass
    /// is the re-embed job for the now-stale vectors.
    pub fn spawn_backfill(self: &Arc<Self>) {
        let sched = self.clone();
        tokio::spawn(async move {
            sched.backfill_with_retry().await;
            let mut tick = tokio::time::interval(Duration::from_secs(BACKFILL_INTERVAL_SECS));
            tick.tick().await; // consume the immediate tick
            let changed = sched.embedder.family_changed.clone();
            loop {
                tokio::select! {
                    _ = tick.tick() => {}
                    _ = changed.notified() => tracing::info!("embedding family changed: re-embedding stale vectors"),
                }
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
    /// Deliberately IDF-ungated: this is the safety net that drains
    /// `pending_embeddings`. It is NOT trust-ungated (F-02): gated chunks
    /// are NULL *because* ingest refused them, so an untrusted provider
    /// skips sensitive/pii-labeled chunks (see `backfill_batch`). A failed
    /// batch logs + records a system_event and stops; the caller retries
    /// (see `backfill_with_retry`). Returns `(embedded, failed)`.
    pub async fn backfill_once(&self) -> (usize, bool) {
        let mut embedded = 0usize;
        let mut failed = false;
        if !self.embedder.cloud_trusted() {
            tracing::info!("embedding backfill: provider untrusted — label-gated chunks stay pending");
        }
        let mut cursor = 0i64;
        loop {
            match self.backfill_batch(cursor).await {
                Ok((n, next)) => {
                    embedded += n;
                    let Some(next) = next else { break };
                    cursor = next;
                }
                Err(e) => {
                    failed = true;
                    let pending = pending_embedding_count(&self.data_dir, &self.embedder.configured_family());
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

    /// Batch-selection SQL (F-02): a trusted provider drains every chunk
    /// that needs a vector; an untrusted one excludes sensitive/pii-labeled
    /// chunks via the item→source join (unlabeled/orphan chunks still drain,
    /// matching ingest, where only a sensitive label gates).
    ///
    /// R45.5: "needs a vector" = NULL vector, an unstamped vector (unknown
    /// provenance), or one of `stale` fingerprints outside the configured
    /// family (bound after `?1` = id cursor, `?2` = limit). The id cursor
    /// guarantees progress even if a re-embedded row fails to re-stamp.
    fn backfill_select_sql(trusted: bool, stale: usize) -> String {
        let mut need = String::from("(c.embedding IS NULL OR c.embed_fp IS NULL");
        if stale > 0 {
            // Numbered from ?3: a bare `?` here would reuse index 2 (SQLite
            // numbers anonymous params by the largest index seen so far).
            let list: Vec<String> = (3..3 + stale).map(|i| format!("?{i}")).collect();
            need.push_str(&format!(" OR c.embed_fp IN ({})", list.join(",")));
        }
        need.push(')');
        let gate = if trusted {
            ""
        } else {
            "LEFT JOIN entities e ON e.id = c.entity_id \
             LEFT JOIN ingested_items i ON i.id = COALESCE(c.item_id, e.item_id) \
             LEFT JOIN sources s ON s.id = i.source_id "
        };
        let label = if trusted { "" } else { " AND COALESCE(s.label,'') NOT IN ('sensitive','pii')" };
        format!(
            "SELECT c.id, c.content, c.path FROM chunks c {gate}\
             WHERE c.id > ?1 AND {need}{label} ORDER BY c.id LIMIT ?2"
        )
    }

    /// One batch of chunks needing a vector, after `last_id` (see
    /// `backfill_select_sql`). Stale fingerprints are re-derived per batch
    /// from the configured family, so a model switch mid-pass is honored.
    fn select_backfill_batch(conn: &rusqlite::Connection, trusted: bool, family: &str, last_id: i64) -> Result<Vec<(i64, String, String)>, String> {
        let (_, stale) = crate::embed_fp::partition_stored_fingerprints(conn, family);
        let mut params: Vec<rusqlite::types::Value> = vec![last_id.into(), (BACKFILL_BATCH as i64).into()];
        params.extend(stale.iter().cloned().map(rusqlite::types::Value::Text));
        let mut stmt = conn.prepare(&Self::backfill_select_sql(trusted, stale.len())).map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(params.iter()), |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))
            .map_err(|e| e.to_string())?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    async fn backfill_batch(&self, last_id: i64) -> Result<(usize, Option<i64>), String> {
        let data_dir = self.data_dir.clone();
        // F-02: gated chunks are NULL *because* ingest refused them — NULL
        // alone can't tell them from merely-failed chunks. When the embed
        // provider is untrusted, exclude sensitive/pii-labeled chunks so the
        // backfill doesn't defeat the source label. Ordinary sources still
        // drain ungated (no IDF gate here — that stays the safety net).
        let trusted = self.embedder.cloud_trusted();
        let family = self.embedder.configured_family();
        let batch: Vec<(i64, String, String)> =
            run_db(&data_dir, move |conn| Self::select_backfill_batch(conn, trusted, &family, last_id)).await?;
        if batch.is_empty() {
            return Ok((0, None));
        }
        let next_cursor = batch.last().map(|(id, _, _)| *id);
        let next_cursor = if batch.len() == BACKFILL_BATCH { next_cursor } else { None };
        // Same breadcrumb-prefixed input as the gated paths — one vector space.
        // R43.6: plus the nomic-gated document prefix, matching `embed_gated`,
        // so backfilled rows land in the same space as ingest-time rows.
        let texts: Vec<String> = batch.iter().map(|(_, c, p)| self.embedder.prefix_document(&crate::markdown::embed_input(p, c))).collect();
        let vectors = self.embedder.embed(texts).await?;
        if vectors.len() != batch.len() {
            return Err(format!("embedder returned {} vectors for {} chunks", vectors.len(), batch.len()));
        }
        let updates: Vec<(i64, Vec<u8>)> = batch.iter().map(|(id, _, _)| *id).zip(vectors).map(|(id, v)| (id, pack_single(&v))).collect();
        let embedded = updates.len();
        let embedder = self.embedder.clone();
        run_db(&data_dir, move |conn| {
            for (cid, blob) in &updates {
                conn.execute("UPDATE chunks SET embedding=?1 WHERE id=?2", rusqlite::params![blob, cid]).map_err(|e| e.to_string())?;
                // R45.5: also stamps `embed_fp` (overwriting a stale one).
                let _ = embedder.sync_vec(conn, *cid, blob);
            }
            Ok(())
        })
        .await?;
        Ok((embedded, next_cursor))
    }
}

/// Chunks still needing a vector: NULL, unstamped, or (R45.5) stamped
/// outside the configured family.
fn pending_embedding_count(data_dir: &str, family: &str) -> i64 {
    let conn = crate::common::open_data_db(data_dir);
    let missing: i64 = conn
        .query_row("SELECT COUNT(*) FROM chunks WHERE embedding IS NULL OR embed_fp IS NULL", [], |r| r.get(0))
        .unwrap_or(0);
    let (_, stale) = crate::embed_fp::partition_stored_fingerprints(&conn, family);
    if stale.is_empty() {
        return missing;
    }
    let sql = format!("SELECT COUNT(*) FROM chunks WHERE embed_fp IN ({})", crate::common::placeholders(stale.len()));
    missing + conn.query_row(&sql, rusqlite::params_from_iter(stale.iter()), |r| r.get::<_, i64>(0)).unwrap_or(0)
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

    /// F-02 trust tests share one DB shape: one ordinary + one sensitive
    /// source, each with a single NULL-embedding chunk.
    fn seed_mixed_labels(dir: &tempfile::TempDir) -> (rusqlite::Connection, i64, i64) {
        let db_path = crate::db::resolve_db_path(&dir.path().to_string_lossy().to_string());
        let conn = crate::db::init_db(&db_path).unwrap();
        conn.execute("INSERT INTO sources (name, connector, config, label) VALUES ('ord','folder','{}','internal')", []).unwrap();
        let ord = conn.last_insert_rowid();
        conn.execute("INSERT INTO sources (name, connector, config, label) VALUES ('sen','folder','{}','sensitive')", []).unwrap();
        let sen = conn.last_insert_rowid();
        for (sid, ext) in [(ord, "o.md"), (sen, "s.md")] {
            conn.execute("INSERT INTO ingested_items (source_id, external_id, title, text, content_hash) VALUES (?1,?2,'t','x','h')", rusqlite::params![sid, ext]).unwrap();
            let iid = conn.last_insert_rowid();
            conn.execute("INSERT INTO chunks (item_id, kind, source_ref, content) VALUES (?1,'document','r','backfill label gate check content')", [iid]).unwrap();
        }
        (conn, ord, sen)
    }

    fn seed_single_label(dir: &tempfile::TempDir, label: &str) -> rusqlite::Connection {
        let db_path = crate::db::resolve_db_path(&dir.path().to_string_lossy().to_string());
        let conn = crate::db::init_db(&db_path).unwrap();
        conn.execute("INSERT INTO sources (name, connector, config, label) VALUES ('s','folder','{}',?1)", [label]).unwrap();
        let sid = conn.last_insert_rowid();
        conn.execute("INSERT INTO ingested_items (source_id, external_id, title, text, content_hash) VALUES (?1,'a.md','t','x','h')", [sid]).unwrap();
        let iid = conn.last_insert_rowid();
        conn.execute("INSERT INTO chunks (item_id, kind, source_ref, content) VALUES (?1,'document','r','backfill label gate check content')", [iid]).unwrap();
        conn
    }

    fn backfill_sched(dir: &tempfile::TempDir, conn: &rusqlite::Connection, embed_base_url: &str) -> (std::sync::Arc<Scheduler>, std::sync::Arc<crate::embedder::Embedder>) {
        let store = crate::secrets::SecretStore::new(dir.path().join("secrets.enc"));
        let svc = std::sync::Arc::new(crate::settings::SettingsService::new(store));
        svc.set_data_dir(dir.path());
        svc.set("embed_base_url", embed_base_url, conn).unwrap();
        let embedder = std::sync::Arc::new(crate::embedder::Embedder::new(svc.clone()));
        let sched = Scheduler::new(JobManager::new(), dir.path().to_string_lossy().to_string(), svc, embedder.clone());
        (sched, embedder)
    }

    /// Hold while a test depends on provider trust: ambient
    /// ANCHOR_CLOUD_TRUST/TRUSTED_PROVIDERS on a dev machine must not flip
    /// the branch. Restores on drop while the lock is still held (fields
    /// drop after `drop()` runs, `_lock` last).
    struct TrustEnvGuard {
        prev_trust: Option<String>,
        prev_allow: Option<String>,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl TrustEnvGuard {
        fn clean() -> Self {
            let lock = crate::common::TRUST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let prev_trust = std::env::var("ANCHOR_CLOUD_TRUST").ok();
            let prev_allow = std::env::var("ANCHOR_TRUSTED_PROVIDERS").ok();
            std::env::remove_var("ANCHOR_CLOUD_TRUST");
            std::env::remove_var("ANCHOR_TRUSTED_PROVIDERS");
            Self { prev_trust, prev_allow, _lock: lock }
        }
    }

    impl Drop for TrustEnvGuard {
        fn drop(&mut self) {
            match &self.prev_trust {
                Some(v) => std::env::set_var("ANCHOR_CLOUD_TRUST", v),
                None => std::env::remove_var("ANCHOR_CLOUD_TRUST"),
            }
            match &self.prev_allow {
                Some(v) => std::env::set_var("ANCHOR_TRUSTED_PROVIDERS", v),
                None => std::env::remove_var("ANCHOR_TRUSTED_PROVIDERS"),
            }
        }
    }

    #[test]
    fn backfill_select_sql_excludes_gated_labels_when_untrusted() {
        // F-02 predicate level: the untrusted batch query must return only
        // the ordinary chunk; the trusted query returns both.
        let dir = tempdir().unwrap();
        let (conn, ord, sen) = seed_mixed_labels(&dir);
        let run = |trusted: bool| {
            Scheduler::select_backfill_batch(&conn, trusted, "nomic-embed-text-v1.5", 0)
                .unwrap()
                .into_iter()
                .map(|(id, _, _)| id)
                .collect::<Vec<_>>()
        };
        let ord_chunk: i64 = conn.query_row("SELECT c.id FROM chunks c JOIN ingested_items i ON i.id=c.item_id WHERE i.source_id=?1", [ord], |r| r.get(0)).unwrap();
        let sen_chunk: i64 = conn.query_row("SELECT c.id FROM chunks c JOIN ingested_items i ON i.id=c.item_id WHERE i.source_id=?1", [sen], |r| r.get(0)).unwrap();
        assert_eq!(run(false), vec![ord_chunk]);
        assert_eq!(run(true), vec![ord_chunk, sen_chunk]);
    }

    #[tokio::test]
    async fn backfill_once_skips_gated_without_request() {
        // F-02 wiring level: sensitive-only DB + untrusted provider. No mock
        // server exists — any HTTP attempt fails — so (0, false) proves the
        // gated chunk was never selected, and it stays NULL.
        let _tg = TrustEnvGuard::clean();
        let dir = tempdir().unwrap();
        let conn = seed_single_label(&dir, "sensitive");
        let (sched, embedder) = backfill_sched(&dir, &conn, "https://untrusted.example.invalid/");
        assert!(!embedder.cloud_trusted());
        let (embedded, failed) = sched.backfill_once().await;
        assert_eq!((embedded, failed), (0, false));
        let pending: i64 = conn.query_row("SELECT COUNT(*) FROM chunks WHERE embedding IS NULL", [], |r| r.get(0)).unwrap();
        assert_eq!(pending, 1, "sensitive chunk must stay NULL");
    }

    #[tokio::test]
    async fn backfill_once_attempts_ordinary_on_untrusted() {
        // F-02 ordinary half: an ordinary chunk under an untrusted provider
        // must still be selected — the attempt against a closed port fails,
        // and that failure is the proof of selection. (A live end-to-end
        // embed under an untrusted URL is untestable hermetically: any
        // reachable test server is loopback, which trust blesses.)
        let _tg = TrustEnvGuard::clean();
        let dir = tempdir().unwrap();
        let conn = seed_single_label(&dir, "internal");
        let (sched, embedder) = backfill_sched(&dir, &conn, "http://0.0.0.0:9");
        assert!(!embedder.cloud_trusted());
        let (embedded, failed) = sched.backfill_once().await;
        assert_eq!(embedded, 0);
        assert!(failed, "ordinary chunk must be attempted (closed port => failure)");
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

    #[tokio::test]
    async fn backfill_reembeds_stale_family_and_restamps() {
        // R45.5: a vector from another model is stale under the configured
        // (default nomic) embedder — the backfill re-embeds and re-stamps it,
        // and leaves an in-family vector alone.
        let dir = tempdir().unwrap();
        let (conn, _sid) = seed_one_item_with_chunks(&dir, &["old model chunk", "current model chunk"]);
        let ids: Vec<i64> = conn.prepare("SELECT id FROM chunks ORDER BY id").unwrap().query_map([], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect();
        let blob = pack_single(&[1.0, 0.0]);
        conn.execute("UPDATE chunks SET embedding=?1, embed_fp='ollama:mxbai-embed-large:2' WHERE id=?2", rusqlite::params![blob, ids[0]]).unwrap();
        // builtin nomic shares the Ollama nomic family: not stale.
        conn.execute("UPDATE chunks SET embedding=?1, embed_fp='builtin:nomic-embed-text-v1.5:2' WHERE id=?2", rusqlite::params![blob, ids[1]]).unwrap();
        let (port, _hits) = mock_embed_server().await;
        let (sched, _embedder) = backfill_sched(&dir, &conn, &format!("http://127.0.0.1:{}", port));
        assert_eq!(pending_embedding_count(&dir.path().to_string_lossy(), "nomic-embed-text-v1.5"), 1);
        let (embedded, failed) = sched.backfill_once().await;
        let detail: String = conn.query_row("SELECT COALESCE(MAX(detail),'') FROM system_events", [], |r| r.get(0)).unwrap();
        assert_eq!((embedded, failed), (1, false), "{detail}");
        let fps: Vec<String> = conn.prepare("SELECT embed_fp FROM chunks ORDER BY id").unwrap().query_map([], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect();
        assert_eq!(fps, vec!["ollama:nomic-embed-text:2".to_string(), "builtin:nomic-embed-text-v1.5:2".to_string()]);
        let vec0: Vec<u8> = conn.query_row("SELECT embedding FROM chunks WHERE id=?1", [ids[0]], |r| r.get(0)).unwrap();
        assert_eq!(vec0, pack_single(&[0.5, 0.25]), "stale vector must be replaced by the mock's");
        assert_eq!(pending_embedding_count(&dir.path().to_string_lossy(), "nomic-embed-text-v1.5"), 0);
    }

    #[tokio::test]
    async fn backfill_unstamped_vector_is_reembedded() {
        // R45.5: a vector with no provenance (old archive, raw write) is
        // unknown — re-embedded and stamped rather than trusted.
        let dir = tempdir().unwrap();
        let (conn, _sid) = seed_one_item_with_chunks(&dir, &["unknown origin chunk"]);
        conn.execute("UPDATE chunks SET embedding=?1", [pack_single(&[1.0, 0.0])]).unwrap();
        let (port, _hits) = mock_embed_server().await;
        let (sched, _embedder) = backfill_sched(&dir, &conn, &format!("http://127.0.0.1:{}", port));
        assert_eq!(sched.backfill_once().await, (1, false));
        let fp: String = conn.query_row("SELECT embed_fp FROM chunks", [], |r| r.get(0)).unwrap();
        assert_eq!(fp, "ollama:nomic-embed-text:2");
    }

    #[tokio::test]
    async fn query_fingerprint_change_records_event_and_wakes_reembed() {
        // R45.5: embed_query records its fingerprint; a family switch
        // records a system event and fires `family_changed`.
        let dir = tempdir().unwrap();
        let (conn, _sid) = seed_one_item_with_chunks(&dir, &["x"]);
        conn.execute("INSERT INTO embed_state (id, active_fp) VALUES (1, 'ollama:mxbai-embed-large:1024')", []).unwrap();
        let (port, _hits) = mock_embed_server().await;
        let (_sched, embedder) = backfill_sched(&dir, &conn, &format!("http://127.0.0.1:{}", port));
        let q = embedder.embed_query("billing").await;
        assert_eq!(q, Some(vec![0.5, 0.25]));
        assert_eq!(crate::embed_fp::active_fingerprint(&conn).as_deref(), Some("ollama:nomic-embed-text:2"));
        let msg: String = conn
            .query_row("SELECT message FROM system_events WHERE component='embedder' ORDER BY id DESC LIMIT 1", [], |r| r.get(0))
            .unwrap();
        assert!(msg.contains("embedding model changed"), "event: {}", msg);
        tokio::time::timeout(Duration::from_secs(1), embedder.family_changed.notified())
            .await
            .expect("family change must wake the re-embed");
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
