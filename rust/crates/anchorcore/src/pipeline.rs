//! Ingestion pipeline — port of `backend/app/pipeline.py:1`.
//! Folder → chunk → classify (LLM + per-doc-type prompt, rules fallback) → distill → embed.

use std::sync::Arc;

use rusqlite::Connection;

use crate::classifier::Classifier;
use crate::connectors::{folder::{FolderConnector, ListedFile}, gdrive::GDriveConnector, jira::JiraConnector, linear::LinearConnector, rest::RestConnector, IngestionDoc};
use crate::embedder::Embedder;
use crate::secrets::SecretStore;
use crate::settings::SettingsService;

fn content_hash(text: &str) -> String {
    crate::hashing::content_hash(text)
}

/// IDF-gate threshold for embedding (env override like the Python engine).
fn embed_min_signal() -> f64 {
    std::env::var("ANCHOR_EMBED_MIN_SIGNAL").ok().and_then(|v| v.parse().ok()).unwrap_or(0.15)
}

fn resolve_config(data_dir: &str, source_id: i64, mut config: serde_json::Value) -> serde_json::Value {
    let store = SecretStore::new(std::path::PathBuf::from(data_dir).join("secrets.enc"));
    if let Some(obj) = config.as_object_mut() {
        for &field in crate::secrets::SECRET_SOURCE_FIELDS {
            if let Some(v) = obj.get(field).and_then(|x| x.as_str()) {
                if v == "***set***" {
                    if let Some(real) = store.get(&format!("source:{}:{}", source_id, field)) {
                        obj.insert(field.to_string(), serde_json::Value::String(real));
                    }
                }
            }
        }
        // linear uses api_key which is already in SECRET_SOURCE_FIELDS, but also handle "token" alias
        if obj.get("api_key").and_then(|v| v.as_str()) == Some("***set***") {
            if let Some(real) = store.get(&format!("source:{}:api_key", source_id))
                .or_else(|| store.get(&format!("source:{}:token", source_id)))
            {
                obj.insert("api_key".to_string(), serde_json::Value::String(real.clone()));
                obj.insert("token".to_string(), serde_json::Value::String(real));
            }
        }
    }
    config
}
fn window_hash(text: &str) -> String {
    crate::hashing::window_hash(text)
}

fn cloud_trusted(settings: &SettingsService, kind: &str) -> bool {
    let base = match kind {
        "classifier" => settings.get("classifier_base_url", None).or_else(|| settings.get("ollama_base_url", None)).unwrap_or_else(|| "http://localhost:11434".to_string()),
        "embed" => settings.get("embed_base_url", None).or_else(|| settings.get("ollama_base_url", None)).unwrap_or_else(|| "http://localhost:11434".to_string()),
        _ => settings.get("ollama_base_url", None).unwrap_or_else(|| "http://localhost:11434".to_string()),
    };
    crate::common::provider_trusted(&base)
}
fn sensitive_label(label: &str) -> bool {
    label == "sensitive" || label == "pii"
}

fn record_sync_error(conn: &Connection, source_id: i64, err: &str) {
    let redacted = err.chars().take(1000).collect::<String>();
    let _ = conn.execute("UPDATE sources SET last_error = ?1, error_count = COALESCE(error_count,0)+1 WHERE id = ?2", rusqlite::params![redacted, source_id]);
    let msg = format!("sync failed for source {}", source_id);
    let _ = conn.execute("INSERT INTO system_events (component, level, source_id, message, detail) VALUES ('pipeline','error',?1,?2,?3)", rusqlite::params![source_id, msg, redacted]);
}
fn record_sync_success(conn: &Connection, source_id: i64) {
    let _ = conn.execute("UPDATE sources SET last_error = NULL, error_count = 0, last_synced_at = datetime('now') WHERE id = ?1", [source_id]);
}

/// Catcher visibility: a PDF whose repair pass strictly improved the text
/// gets one warning row. Ties stay debug-level — foreign text and typos
/// trip the detector without being extractable damage.
async fn record_pdf_quality(data_dir: &str, source_id: i64, detail: String) {
    let _ = run_db(data_dir, move |conn| {
        let _ = conn.execute(
            "INSERT INTO system_events (component, level, source_id, message, detail) VALUES ('pipeline','warning',?1,'pdf split-word repair',?2)",
            rusqlite::params![source_id, detail],
        );
        Ok::<(), String>(())
    })
    .await;
}
// R13.1: single DB-open boilerplate — every spawn_blocking DB task goes through this.
fn with_db<T>(data_dir: &str, f: impl FnOnce(&Connection) -> T) -> T {
    let db_path = crate::db::resolve_db_path(data_dir);
    let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| Connection::open(&db_path).unwrap());
    f(&conn)
}

pub(crate) async fn run_db<T, F>(data_dir: &str, f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&Connection) -> Result<T, String> + Send + 'static,
{
    let data_dir = data_dir.to_string();
    tokio::task::spawn_blocking(move || with_db(&data_dir, f)).await.map_err(|e| e.to_string())?
}

async fn job_status(data_dir: &str, job_id: i64) -> Result<String, String> {
    run_db(data_dir, move |conn| {
        conn.query_row("SELECT status FROM jobs WHERE id=?1", [job_id], |r| r.get(0)).map_err(|e| e.to_string())
    }).await
}

async fn job_cancelled(data_dir: &str, job_id: i64) -> Result<bool, String> {
    Ok(job_status(data_dir, job_id).await? == "cancelled")
}

/// Returns `(docs, skipped)` — `skipped` are files present at the source but
/// unreadable (folder connector), kept out of purge (B57 safety).
async fn fetch_docs(connector_type: &str, config: &serde_json::Value, source_id: i64) -> Result<(Vec<IngestionDoc>, Vec<String>), String> {
    // R10.2: Jira/GDrive/Linear must not silently succeed with 0 items
    let (docs, skipped) = match connector_type {
        "folder" => {
            let fc = FolderConnector::new(config).map_err(|e| e.to_string())?;
            let (docs, skipped, _cursor) = fc.fetch_with_skipped().map_err(|e| e.to_string())?;
            (docs, skipped)
        }
        "jira" => {
            let jc = JiraConnector::new(config).map_err(|e| e.to_string())?;
            let (docs, _cursor) = jc.fetch("").await.map_err(|e| e.to_string())?;
            if docs.is_empty() {
                // treat empty as not error for now, but log
                tracing::warn!("Jira fetch returned 0 docs for source {}", source_id);
            }
            (docs, Vec::new())
        }
        "gdrive" => {
            let gc = GDriveConnector::new(config).map_err(|e| e.to_string())?;
            let (docs, skipped, _cursor) = gc.fetch_with_skipped("").await.map_err(|e| e.to_string())?;
            (docs, skipped)
        }
        "linear" => {
            let lc = LinearConnector::new(config).map_err(|e| e.to_string())?;
            let (docs, _cursor) = lc.fetch("").await.map_err(|e| e.to_string())?;
            (docs, Vec::new())
        }
        "rest" => {
            let rc = RestConnector::new(config).map_err(|e| e.to_string())?;
            let (docs, _cursor) = rc.fetch("").await.map_err(|e| e.to_string())?;
            if docs.is_empty() {
                tracing::warn!("REST fetch returned 0 docs for source {}", source_id);
            }
            (docs, Vec::new())
        }
        _ => return Err(format!("unknown connector '{}'", connector_type)),
    };
    Ok((docs, skipped))
}

/// B57: connectors whose fetch returns the complete current set, so a stored
/// item missing from fetch means "deleted remotely". `rest` is excluded:
/// `max_pages` can truncate the list, and purging on a partial fetch would
/// delete live items.
fn connector_supports_purge(connector_type: &str) -> bool {
    matches!(connector_type, "folder" | "jira" | "gdrive" | "linear")
}

/// B57: hard-delete stored items absent from `fetched`. The source is the
/// source of truth (a rename already reads as delete+add). FK cascades +
/// chunks triggers clean entities/chunks/vec/fts/sections; only the shared
/// `tags.count` needs a recompute in `purge_item`.
async fn purge_missing_items(
    data_dir: &str,
    source_id: i64,
    fetched: std::collections::HashSet<String>,
) -> Result<i64, String> {
    run_db(data_dir, move |conn| purge_missing_inner(conn, source_id, &fetched)).await
}

/// B57: sync-time entry point. Purge is cleanup — a failure must not fail
/// the whole sync, so errors are logged and count as zero. Empty fetches
/// purge nothing (a misconfiguration must not wipe the index). Extracted
/// so `run_sync_inner` stays at its CCN budget.
async fn purge_missing_for_sync(
    data_dir: &str,
    source_id: i64,
    connector_type: &str,
    docs: &[IngestionDoc],
    skipped: &[String],
) -> i64 {
    if !connector_supports_purge(connector_type) {
        return 0;
    }
    let mut fetched: std::collections::HashSet<String> =
        docs.iter().map(|d| d.external_id.clone()).collect();
    // unreadable-but-present files still exist at the source — never purge them
    fetched.extend(skipped.iter().cloned());
    purge_fetched_set(data_dir, source_id, fetched).await
}

/// Purge stored items absent from a complete `fetched` set. Cleanup only — a
/// failure logs and counts as zero, and an empty set purges nothing (a
/// misconfiguration must not wipe the index).
async fn purge_fetched_set(
    data_dir: &str,
    source_id: i64,
    fetched: std::collections::HashSet<String>,
) -> i64 {
    if fetched.is_empty() {
        return 0;
    }
    match purge_missing_items(data_dir, source_id, fetched).await {
        Ok(n) => n,
        Err(e) => {
            tracing::warn!("purge failed for source {}: {}", source_id, e);
            0
        }
    }
}

fn purge_missing_inner(
    conn: &Connection,
    source_id: i64,
    fetched: &std::collections::HashSet<String>,
) -> Result<i64, String> {
    let stored: Vec<(i64, String)> = conn
        .prepare("SELECT id, external_id FROM ingested_items WHERE source_id=?1")
        .map_err(|e| e.to_string())?
        .query_map([source_id], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();
    let mut purged = 0i64;
    for (id, external_id) in stored {
        if fetched.contains(&external_id) {
            continue;
        }
        purge_item(conn, id)?;
        purged += 1;
    }
    Ok(purged)
}

/// Live job progress: `processed` counts files scanned, `result` carries the
/// changed/unchanged-so-far counts in the final result's shape, so progress
/// reads "200/514 · 3 changed" instead of implying a full re-sync.
async fn update_sync_progress(data_dir: &str, job_id: i64, processed: i64, items: i64, entities: i64, purged: i64, unchanged: i64) {
    let result = serde_json::json!({"items": items, "entities": entities, "purged": purged, "unchanged": unchanged}).to_string();
    let _ = run_db(data_dir, move |conn| {
        let _ = conn.execute("UPDATE jobs SET processed=?1, result=?2 WHERE id=?3", rusqlite::params![processed, result, job_id]);
        Ok(())
    })
    .await;
}

/// Stored stat fingerprints for a folder source: `external_id -> (mtime_ms, len)`.
/// Rows without one (other connectors, pre-migration rows) are absent — the
/// caller always extracts those.
async fn load_fingerprints(data_dir: &str, source_id: i64) -> std::collections::HashMap<String, (u64, u64)> {
    run_db(data_dir, move |conn| {
        let mut stmt = conn
            .prepare("SELECT external_id, file_mtime_ms, file_size FROM ingested_items WHERE source_id=?1 AND file_mtime_ms IS NOT NULL AND file_size IS NOT NULL")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([source_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?)))
            .map_err(|e| e.to_string())?;
        let mut out = std::collections::HashMap::new();
        for row in rows.filter_map(|r| r.ok()) {
            out.insert(row.0, (row.1 as u64, row.2 as u64));
        }
        Ok(out)
    })
    .await
    .unwrap_or_default()
}

/// Persist a file's stat fingerprint alongside its content (`None` leaves the
/// columns untouched — only folder syncs set them).
fn store_fingerprint(conn: &Connection, item_id: i64, fp: Option<(u64, u64)>) {
    if let Some((mtime_ms, len)) = fp {
        let _ = conn.execute(
            "UPDATE ingested_items SET file_mtime_ms=?1, file_size=?2 WHERE id=?3",
            rusqlite::params![mtime_ms as i64, len as i64, item_id],
        );
    }
}

fn purge_item(conn: &Connection, item_id: i64) -> Result<(), String> {
    let tag_ids: Vec<i64> = conn
        .prepare(
            "SELECT DISTINCT tag_id FROM chunk_tags WHERE chunk_id IN (
                 SELECT id FROM chunks WHERE item_id=?1
                 UNION
                 SELECT id FROM chunks WHERE entity_id IN (SELECT id FROM entities WHERE item_id=?1))",
        )
        .map_err(|e| e.to_string())?
        .query_map([item_id], |r| r.get(0))
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();
    conn.execute("DELETE FROM ingested_items WHERE id=?1", [item_id])
        .map_err(|e| e.to_string())?;
    for tag_id in tag_ids {
        conn.execute(
            "UPDATE tags SET count = (SELECT COUNT(*) FROM chunk_tags WHERE tag_id=?1) WHERE id=?1",
            [tag_id],
        )
        .map_err(|e| e.to_string())?;
    }
    let _ = conn.execute("DELETE FROM tags WHERE count <= 0", []);
    Ok(())
}

/// What one folder file cost the sync. Only `Skipped` proves the fingerprint
/// tier worked (the file was never extracted); `NoChange` covers both
/// extracted-but-identical and unreadable files.
enum FileOutcome {
    Skipped,
    NoChange,
    Classified(usize),
}

pub struct Pipeline {
    pub classifier: Arc<Classifier>,
    pub embedder: Arc<Embedder>,
    pub settings: Arc<SettingsService>,
    pub data_dir: String,
    pub jobs: Arc<crate::jobs::JobManager>,
}

impl Pipeline {
    pub fn new(classifier: Arc<Classifier>, embedder: Arc<Embedder>, settings: Arc<SettingsService>, data_dir: String, jobs: Arc<crate::jobs::JobManager>) -> Self {
        Self { classifier, embedder, settings, data_dir, jobs }
    }
    // legacy new for tests (creates dummy jobs)
    pub fn new_for_test(classifier: Arc<Classifier>, embedder: Arc<Embedder>, settings: Arc<SettingsService>, data_dir: String) -> Self {
        Self { classifier, embedder, settings, data_dir, jobs: crate::jobs::JobManager::new() }
    }

    pub async fn sync_source(&self, source_id: i64, job_id: i64, force_reclassify: bool) {
        let data_dir = self.data_dir.clone();
        if !self.wait_for_slot(&data_dir, job_id).await {
            return;
        }
        let res = Box::pin(self.run_sync_inner(source_id, job_id, force_reclassify, &data_dir)).await;
        let conn = crate::common::open_data_db(&data_dir);
        self.finish_job(&conn, source_id, job_id, res);
    }

    /// R25.3: embed every chunk of a source that has no vector — imported
    /// archives leave embeddings out by default, and retrieval honestly reports
    /// keyword-only (B58) until this finishes. Walks the chunk ids in ascending
    /// order so a gated batch can never spin the loop.
    pub async fn embed_missing_for_source(&self, source_id: i64, job_id: i64) {
        let data_dir = self.data_dir.clone();
        if !self.wait_for_slot(&data_dir, job_id).await {
            return;
        }
        let res = Box::pin(self.run_embed_missing(source_id, job_id, &data_dir)).await;
        let conn = crate::common::open_data_db(&data_dir);
        self.finish_job(&conn, source_id, job_id, res);
    }

    async fn run_embed_missing(&self, source_id: i64, job_id: i64, data_dir: &str) -> Result<(i64, i64, i64, i64), String> {
        let label: String = run_db(data_dir, move |conn| {
            conn.query_row("SELECT label FROM sources WHERE id=?1", [source_id], |r| r.get(0))
                .map_err(|e| e.to_string())
        })
        .await?;
        let gated = sensitive_label(&label) && !cloud_trusted(&self.settings, "embed");
        let mut processed = 0i64;
        let mut last_id = 0i64;
        loop {
            if job_cancelled(data_dir, job_id).await? {
                return Err("job cancelled".to_string());
            }
            let batch = fetch_embed_batch(data_dir, source_id, last_id).await?;
            let Some(last) = batch.last().map(|(id, _)| *id) else {
                break;
            };
            last_id = last;
            processed += batch.len() as i64;
            let count = processed;
            let _ = run_db(data_dir, move |conn| {
                conn.execute("UPDATE jobs SET processed=?1 WHERE id=?2", rusqlite::params![count, job_id])
                    .map_err(|e| e.to_string())?;
                Ok(())
            })
            .await;
            let contents: Vec<String> = batch.iter().map(|(_, c)| c.clone()).collect();
            let out = self
                .embedder
                .embed_gated(contents, gated, embed_min_signal())
                .await
                .unwrap_or_else(|_| vec![None; batch.len()]);
            let updates: Vec<(i64, Vec<u8>)> = batch
                .iter()
                .zip(out)
                .filter_map(|((id, _), blob)| blob.map(|b| (*id, b)))
                .collect();
            if updates.is_empty() {
                continue;
            }
            store_embeddings(data_dir, self.embedder.clone(), updates).await?;
        }
        // Embed jobs gained no `total` while walking; close it so the UI shows
        // processed/total instead of N/0 (finish_job no longer back-fills it).
        let total = processed;
        let _ = run_db(data_dir, move |conn| {
            let _ = conn.execute("UPDATE jobs SET total=?1 WHERE id=?2", rusqlite::params![total, job_id]);
            Ok(())
        })
        .await;
        Ok((processed, 0, 0, 0))
    }

    /// R10.4: respect bounded concurrency — do NOT flip pending→running here.
    /// JobManager::maybe_promote is the sole owner of pending→running.
    /// Returns true once the job is running and owned; false when there is
    /// nothing to do (cancelled/failed/done — promotion already handled).
    async fn wait_for_slot(&self, data_dir: &str, job_id: i64) -> bool {
        let db_path = crate::db::resolve_db_path(data_dir);
        let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| Connection::open(&db_path).unwrap());
        let status: String = conn.query_row("SELECT status FROM jobs WHERE id=?1", [job_id], |r| r.get(0)).unwrap_or_else(|_| "pending".to_string());
        if status == "cancelled" {
            self.jobs.maybe_promote(&conn);
            return false;
        }
        if status == "pending" {
            drop(conn);
            // wait for promotion (MAX_CONCURRENT gate)
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                let db_path2 = crate::db::resolve_db_path(data_dir);
                let conn2 = crate::db::init_db(&db_path2).unwrap_or_else(|_| Connection::open(&db_path2).unwrap());
                let cur: String = conn2.query_row("SELECT status FROM jobs WHERE id=?1", [job_id], |r| r.get(0)).unwrap_or_else(|_| "pending".to_string());
                if cur == "cancelled" {
                    crate::jobs::JobManager::new().maybe_promote(&conn2);
                    return false;
                }
                if cur == "running" {
                    break;
                }
                if cur != "pending" {
                    // failed/done — should not happen for pending waiter, just exit
                    return false;
                }
            }
        } else if status != "running" {
            // unexpected state (failed/done) — nothing to do
            return false;
        }
        // job is now running (promoted by JobManager); proceed without extra UPDATE
        true
    }

    /// Record the sync outcome without clobbering a concurrent cancel (R9.1),
    /// then promote the next pending job.
    fn finish_job(&self, conn: &Connection, source_id: i64, job_id: i64, res: Result<(i64, i64, i64, i64), String>) {
        match res {
            Ok((items, entities, purged, unchanged)) => {
                // R9.1: do not clobber concurrent cancel
                let cur: String = conn.query_row("SELECT status FROM jobs WHERE id=?1", [job_id], |r| r.get(0)).unwrap_or_else(|_| "done".to_string());
                if cur == "cancelled" {
                    self.jobs.maybe_promote(conn);
                } else {
                    let result = serde_json::json!({"items": items, "entities": entities, "purged": purged, "unchanged": unchanged}).to_string();
                    let _ = conn.execute("UPDATE jobs SET status='done', finished_at=datetime('now'), result=?1, error=NULL WHERE id=?2", rusqlite::params![result, job_id]);
                    record_sync_success(conn, source_id);
                    // R28.1: keep the totals set by the run phase (total = docs
                    // scanned, processed = docs walked) — `items` is only the
                    // changed/new subset and used to overwrite both, making a
                    // 10-file resync with 3 changed files read as 3/3.
                    // promote next pending
                    self.jobs.maybe_promote(conn);
                }
            }
            Err(e) if e == "job cancelled" => {
                // R9.1: cancelled must stay cancelled, no error_count bump, promote next
                let _ = conn.execute("UPDATE jobs SET status='cancelled', finished_at=datetime('now'), error='job cancelled' WHERE id=?1", [job_id]);
                self.jobs.maybe_promote(conn);
            }
            Err(e) => {
                // do not clobber a concurrent cancel that already set cancelled
                let cur: String = conn.query_row("SELECT status FROM jobs WHERE id=?1", [job_id], |r| r.get(0)).unwrap_or_else(|_| "failed".to_string());
                if cur == "cancelled" {
                    self.jobs.maybe_promote(conn);
                } else {
                    let _ = conn.execute("UPDATE jobs SET status='failed', finished_at=datetime('now'), error=?1 WHERE id=?2", rusqlite::params![e, job_id]);
                    record_sync_error(conn, source_id, &e);
                    self.jobs.maybe_promote(conn);
                }
            }
        }
    }

    async fn run_sync_inner(&self, source_id: i64, job_id: i64, force_reclassify: bool, data_dir: &str) -> Result<(i64, i64, i64, i64), String> {
        // load source
        let (connector_type, config_str) = run_db(data_dir, move |conn| {
            let mut stmt = conn.prepare("SELECT connector, config FROM sources WHERE id = ?1").map_err(|e| e.to_string())?;
            stmt.query_row([source_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))).map_err(|_| "Source not found".to_string())
        }).await?;
        let mut config: serde_json::Value = serde_json::from_str(&config_str).unwrap_or(serde_json::json!({}));
        config = resolve_config(data_dir, source_id, config);
        // Folder sync is incremental (stat listing + stored fingerprints decide
        // *whether to extract*); API connectors fetch everything and hash-skip.
        if connector_type == "folder" {
            return self.run_folder_sync(source_id, job_id, force_reclassify, &config, data_dir).await;
        }
        // fetch docs — R10.2: Jira/GDrive/Linear must not silently succeed with 0 items
        let t_fetch = std::time::Instant::now();
        let (docs, skipped) = fetch_docs(&connector_type, &config, source_id).await?;
        tracing::debug!(target: "anchorcore::perf", docs = docs.len(), ms = t_fetch.elapsed().as_millis(), "fetch");
        // B57: purge stored items that disappeared remotely (hard-delete).
        let purged_items =
            purge_missing_for_sync(data_dir, source_id, &connector_type, &docs, &skipped).await;
        // update job total
        let total_docs = docs.len() as i64;
        run_db(data_dir, move |conn| {
            let _ = conn.execute("UPDATE jobs SET total=?1 WHERE id=?2", rusqlite::params![total_docs, job_id]);
            Ok(())
        }).await?;
        let mut created_items = 0i64;
        let mut new_entities = 0i64;
        let mut unchanged = 0i64;
        for (idx, doc) in docs.into_iter().enumerate() {
            // check if job was cancelled
            if job_cancelled(data_dir, job_id).await? {
                return Err("job cancelled".to_string());
            }
            match Box::pin(self.sync_fetched_doc(data_dir, source_id, job_id, &doc, force_reclassify)).await? {
                Some(n) => {
                    created_items += 1;
                    new_entities += n as i64;
                }
                None => unchanged += 1,
            }
            update_sync_progress(data_dir, job_id, (idx + 1) as i64, created_items, new_entities, purged_items, unchanged).await;
        }
        Ok((created_items, new_entities, purged_items, unchanged))
    }

    /// One fetched API doc: hash-skip or classify. Returns `Some(n)` when
    /// (re)classified with N entities.
    async fn sync_fetched_doc(&self, data_dir: &str, source_id: i64, job_id: i64, doc: &IngestionDoc, force: bool) -> Result<Option<usize>, String> {
        let upserted = Box::pin(self.upsert_doc(data_dir, source_id, doc, force, None)).await?;
        if upserted || force {
            let n = Box::pin(self.classify_and_store(data_dir, source_id, job_id, doc, force)).await?;
            return Ok(Some(n));
        }
        Ok(None)
    }

    /// Folder sync that only extracts new/changed files: one cheap `stat`
    /// listing up front, stored fingerprints decide *whether to extract*, and
    /// the content hash stays the authority for *whether to reclassify*.
    /// Unreadable files stay in the listing, so purge can never mistake them
    /// for deleted (B57) — no separate `skipped` set needed.
    async fn run_folder_sync(&self, source_id: i64, job_id: i64, force_reclassify: bool, config: &serde_json::Value, data_dir: &str) -> Result<(i64, i64, i64, i64), String> {
        let fc = FolderConnector::new(config).map_err(|e| e.to_string())?;
        let t_list = std::time::Instant::now();
        let listed = fc.list_files().map_err(|e| e.to_string())?;
        tracing::debug!(target: "anchorcore::perf", files = listed.len(), ms = t_list.elapsed().as_millis(), "list");
        let fetched: std::collections::HashSet<String> = listed.iter().map(|f| f.rel.clone()).collect();
        let purged_items = purge_fetched_set(data_dir, source_id, fetched).await;
        let total = listed.len() as i64;
        run_db(data_dir, move |conn| {
            let _ = conn.execute("UPDATE jobs SET total=?1 WHERE id=?2", rusqlite::params![total, job_id]);
            Ok(())
        }).await?;
        let known = if force_reclassify {
            std::collections::HashMap::new()
        } else {
            load_fingerprints(data_dir, source_id).await
        };
        let mut created_items = 0i64;
        let mut new_entities = 0i64;
        let mut unchanged = 0i64;
        for (idx, f) in listed.iter().enumerate() {
            if job_cancelled(data_dir, job_id).await? {
                return Err("job cancelled".to_string());
            }
            match Box::pin(self.sync_listed_file(data_dir, source_id, job_id, force_reclassify, &known, f)).await? {
                FileOutcome::Classified(n) => {
                    created_items += 1;
                    new_entities += n as i64;
                }
                FileOutcome::Skipped => unchanged += 1,
                FileOutcome::NoChange => {}
            }
            update_sync_progress(data_dir, job_id, (idx + 1) as i64, created_items, new_entities, purged_items, unchanged).await;
        }
        Ok((created_items, new_entities, purged_items, unchanged))
    }

    /// One listed file: skip when the stored fingerprint matches, else extract
    /// (unreadable counts as processed, never purged) and run the shared
    /// upsert → classify tail.
    async fn sync_listed_file(
        &self,
        data_dir: &str,
        source_id: i64,
        job_id: i64,
        force: bool,
        known: &std::collections::HashMap<String, (u64, u64)>,
        f: &ListedFile,
    ) -> Result<FileOutcome, String> {
        let unchanged = known.get(&f.rel).map(|fp| *fp == (f.mtime_ms, f.len)).unwrap_or(false);
        if !force && unchanged {
            return Ok(FileOutcome::Skipped);
        }
        let (extracted, pdf_note) = FolderConnector::extract_listed(f);
        let text = match extracted {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!("folder file skipped ({}): {}", f.rel, e);
                return Ok(FileOutcome::NoChange);
            }
        };
        if let Some(note) = pdf_note.filter(|n| n.used_alternate) {
            record_pdf_quality(data_dir, source_id, note.summary()).await;
        }
        let doc = IngestionDoc {
            external_id: f.rel.clone(),
            title: f.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| f.rel.clone()),
            text,
            author: String::new(),
            updated_at: None,
            source_ref: f.path.to_string_lossy().to_string(),
        };
        let upserted = Box::pin(self.upsert_doc(data_dir, source_id, &doc, force, Some((f.mtime_ms, f.len)))).await?;
        if upserted || force {
            let n = Box::pin(self.classify_and_store(data_dir, source_id, job_id, &doc, force)).await?;
            return Ok(FileOutcome::Classified(n));
        }
        // Touched-but-identical: extracted, hash unchanged — processed, no counts.
        Ok(FileOutcome::NoChange)
    }

    async fn upsert_doc(&self, data_dir: &str, source_id: i64, doc: &IngestionDoc, _force: bool, fp: Option<(u64, u64)>) -> Result<bool, String> {
        let digest = content_hash(&doc.text);
        let doc = doc.clone();
        run_db(data_dir, move |conn| {
            let existing: Option<(i64, String, Option<String>)> = conn.query_row("SELECT id, content_hash, text FROM ingested_items WHERE source_id=?1 AND external_id=?2", rusqlite::params![source_id, doc.external_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).ok();
            if let Some((id, existing_hash, existing_text)) = existing {
                let missing_text = existing_text.as_deref().unwrap_or("").is_empty();
                if existing_hash == digest && !missing_text {
                    // Touched-but-identical: still refresh the stat fingerprint
                    // so the next sync skips extraction instead of retrying it.
                    store_fingerprint(conn, id, fp);
                    return Ok(false);
                }
                let _ = conn.execute("UPDATE ingested_items SET content_hash=?1, title=?2, author=?3, text=?4, stale=0 WHERE id=?5", rusqlite::params![digest, doc.title, doc.author, doc.text, id]);
                store_fingerprint(conn, id, fp);
                Ok(true)
            } else {
                conn.execute("INSERT INTO ingested_items (source_id, external_id, title, text, content_hash, author, stale, created_at, uid) VALUES (?1,?2,?3,?4,?5,?6, 0, datetime('now'),?7)", rusqlite::params![source_id, doc.external_id, doc.title, doc.text, digest, doc.author, crate::db::new_uid()]).map_err(|e| e.to_string())?;
                store_fingerprint(conn, conn.last_insert_rowid(), fp);
                Ok(true)
            }
        }).await
    }

    async fn classify_and_store(&self, data_dir: &str, source_id: i64, job_id: i64, doc: &IngestionDoc, force_reclassify: bool) -> Result<usize, String> {
        let mut perf = PhaseTimes::new();
        let mut ctx = ClassifyCtx::new(data_dir, source_id, job_id, doc, force_reclassify, self.settings.clone(), self.classifier.clone(), self.embedder.clone());
        ctx.load_item_meta().await?;
        perf.mark("meta");
        ctx.detect_doc_type().await?;
        perf.mark("detect");
        // If force, delete all entities for item. Structured files always
        // purge: R33.2/R33.4 skip extraction for code + configs, so a re-sync
        // removes legacy entities (and their chunks via FK cascade) with no
        // data migration.
        if force_reclassify || ctx.skips_extraction() {
            delete_stale_entities(&ctx, None).await?;
        }
        perf.mark("stale");
        // R9.1: abort early if cancelled before windowing
        if job_cancelled(&ctx.data_dir, job_id).await? {
            return Err("job cancelled".to_string());
        }
        // R33.2/R33.4: structured files skip extraction; chunk + embed only.
        // Windows exist solely for classification, so drop them for a truthful log.
        let total_entities = if ctx.skips_extraction() {
            ctx.windows.clear();
            0
        } else {
            ctx.classify_windows().await?
        };
        perf.mark("classify");
        ctx.finalize(&mut perf).await?;
        perf.mark("done");
        tracing::debug!(
            target: "anchorcore::perf",
            file = %ctx.external_id,
            doc_type = %ctx.doc_type,
            chars = ctx.full_text.len(),
            windows = ctx.windows.len(),
            entities = total_entities,
            text = %perf.render(),
            "doc ingest"
        );
        Ok(total_entities)
    }
}

/// R30.2 perf: per-phase ingest timings, debug-logged (RUST_LOG=anchorcore::perf=debug).
struct PhaseTimes {
    rows: Vec<(&'static str, u128)>,
    last: std::time::Instant,
}

impl PhaseTimes {
    fn new() -> Self {
        Self { rows: Vec::new(), last: std::time::Instant::now() }
    }
    fn mark(&mut self, name: &'static str) {
        let now = std::time::Instant::now();
        self.rows.push((name, self.last.elapsed().as_millis()));
        self.last = now;
    }
    fn render(&self) -> String {
        self.rows.iter().map(|(n, ms)| format!("{}={}ms", n, ms)).collect::<Vec<_>>().join(" ")
    }
}

// ---- R13.1: classify_and_store phase helpers ----

struct ClassifyCtx {
    data_dir: String,
    source_id: i64,
    job_id: i64,
    force: bool,
    settings: Arc<SettingsService>,
    classifier: Arc<Classifier>,
    embedder: Arc<Embedder>,
    base_ref: String,
    windows: Vec<String>,
    full_text: String,
    external_id: String,
    item_id: i64,
    classify_trusted: bool,
    embed_trusted: bool,
    doc_type: String,
    label: String,
    prev_hashes: std::collections::HashMap<String, String>,
    new_hashes: std::collections::HashMap<String, String>,
}

impl ClassifyCtx {
    fn new(data_dir: &str, source_id: i64, job_id: i64, doc: &IngestionDoc, force: bool, settings: Arc<SettingsService>, classifier: Arc<Classifier>, embedder: Arc<Embedder>) -> Self {
        let base_ref = if doc.source_ref.is_empty() { doc.title.clone() } else { doc.source_ref.clone() };
        let windows = crate::chunking::classify_windows(&doc.text);
        Self {
            data_dir: data_dir.to_string(),
            source_id,
            job_id,
            force,
            settings,
            classifier,
            embedder,
            base_ref,
            windows,
            full_text: doc.text.clone(),
            external_id: doc.external_id.clone(),
            item_id: 0,
            classify_trusted: false,
            embed_trusted: false,
            doc_type: String::new(),
            label: String::new(),
            prev_hashes: std::collections::HashMap::new(),
            new_hashes: std::collections::HashMap::new(),
        }
    }

    async fn load_item_meta(&mut self) -> Result<(), String> {
        let source_id = self.source_id;
        let external_id = self.external_id.clone();
        let (item_id, label, prev_hashes, doc_type) = run_db(&self.data_dir, move |conn| {
            let item_id: i64 = conn.query_row("SELECT id FROM ingested_items WHERE source_id=?1 AND external_id=?2", rusqlite::params![source_id, external_id], |r| r.get(0)).map_err(|e| e.to_string())?;
            let label: String = conn.query_row("SELECT label FROM sources WHERE id=?1", [source_id], |r| r.get(0)).unwrap_or_else(|_| "internal".to_string());
            let s: String = conn.query_row("SELECT window_hashes FROM ingested_items WHERE id=?1", [item_id], |r| r.get(0)).unwrap_or_else(|_| "{}".to_string());
            let prev_hashes: std::collections::HashMap<String, String> = serde_json::from_str(&s).unwrap_or_default();
            let doc_type: String = conn.query_row("SELECT doc_type FROM ingested_items WHERE id=?1", [item_id], |r| r.get(0)).unwrap_or_default();
            Ok((item_id, label, prev_hashes, doc_type))
        }).await?;
        self.item_id = item_id;
        self.label = label;
        self.prev_hashes = prev_hashes;
        self.doc_type = doc_type;
        self.new_hashes = self.prev_hashes.clone();
        Ok(())
    }

    // B26 document-type pre-pass: one cheap detection per document, cached on
    // the item; force reclassify re-detects (recovers from an earlier bad label).
    async fn detect_doc_type(&mut self) -> Result<(), String> {
        self.classify_trusted = cloud_trusted(&*self.settings, "classifier");
        self.embed_trusted = cloud_trusted(&*self.settings, "embed");
        if !self.doc_type.is_empty() && !self.force {
            return Ok(());
        }
        let trusted = self.classify_trusted;
        // Box LLM future to bound stack (large classifier future previously blew 8 MB)
        // Extension fast path: source files are `code` whatever the content
        // looks like (comments may read like prose) — no LLM call needed.
        let doc_type = if crate::extract::is_code_filename(&self.external_id)
            || crate::extract::is_code_filename(&self.base_ref)
        {
            "code".to_string()
        } else {
            // Box LLM future to bound stack (large classifier future previously blew 8 MB)
            Box::pin(self.classifier.detect_document_type(&self.full_text, trusted)).await
        };
        self.doc_type = doc_type.clone();
        let item_id = self.item_id;
        run_db(&self.data_dir, move |conn| {
            conn.execute("UPDATE ingested_items SET doc_type=?1 WHERE id=?2", rusqlite::params![doc_type, item_id]).map_err(|e| e.to_string())?;
            Ok(())
        }).await
    }

    /// R33.2/R33.4: structured files (source + hierarchical config) are never
    /// entity-extracted — the content itself is the retrievable unit. Configs
    /// keep their detected prose doc_type (truthful badge); only the filename
    /// decides the skip.
    fn skips_extraction(&self) -> bool {
        self.doc_type == "code"
            || crate::extract::is_config_filename(&self.external_id)
            || crate::extract::is_config_filename(&self.base_ref)
    }

    async fn classify_windows(&mut self) -> Result<usize, String> {
        // B26/R22.1: per-window LLM extraction (rules fallback inside
        // `Classifier::classify`), hash-skipped and bounded-concurrency.
        let mut jobs: Vec<(i64, String, String)> = Vec::new();
        for (idx, window) in self.windows.iter().enumerate() {
            let index = (idx + 1) as i64;
            let h = window_hash(window);
            let unchanged = !self.force && self.prev_hashes.get(&index.to_string()).map(|p| *p == h).unwrap_or(false);
            if unchanged {
                self.new_hashes.insert(index.to_string(), h);
                continue;
            }
            self.new_hashes.insert(index.to_string(), h);
            let source_ref = if self.windows.len() > 1 {
                format!("{} §{}", self.base_ref, index)
            } else {
                self.base_ref.clone()
            };
            jobs.push((index, source_ref, window.clone()));
        }
        if jobs.is_empty() {
            return Ok(0);
        }
        if job_cancelled(&self.data_dir, self.job_id).await? {
            return Err("job cancelled".to_string());
        }
        let classified = classify_all(&self.classifier, &jobs, &self.doc_type, self.classify_trusted, &self.data_dir, self.job_id).await?;
        let mut total = 0usize;
        for ((index, source_ref, window), items) in jobs.iter().zip(classified) {
            if job_cancelled(&self.data_dir, self.job_id).await? {
                return Err("job cancelled".to_string());
            }
            // delete stale entities for this window index if not force (already handled force)
            if !self.force {
                delete_stale_entities(self, Some(*index)).await?;
            }
            total += store_batch(self, prepare_items(items, window, source_ref, *index)).await?;
        }
        Ok(total)
    }

    async fn finalize(&self, perf: &mut PhaseTimes) -> Result<(), String> {
        update_window_hashes(self).await?;
        perf.mark("hashes");
        store_doc_chunks(self).await?;
        perf.mark("chunks");
        create_summaries(self).await?;
        perf.mark("summaries");
        create_tags(self).await?;
        perf.mark("tags");
        distill_item(self).await?;
        perf.mark("distill");
        embed_chunks(self).await?;
        perf.mark("embed");
        Ok(())
    }
}

fn truncate_trimmed(s: &str, n: usize) -> String {
    s.chars().take(n).collect::<String>().trim().to_string()
}

fn extractive_section_summary(text: &str) -> String {
    let paras: Vec<&str> = text.split("\n\n").collect();
    let first_two = paras.iter().take(2).cloned().collect::<Vec<_>>().join("\n\n");
    let trimmed = first_two.trim();
    if trimmed.is_empty() {
        return truncate_trimmed(text, 800);
    }
    truncate_trimmed(trimmed, 1200)
}

fn extractive_doc_summary(text: &str) -> String {
    let cleaned = text.trim();
    if cleaned.is_empty() {
        return String::new();
    }
    truncate_trimmed(cleaned, 1200)
}

async fn create_summaries(ctx: &ClassifyCtx) -> Result<(), String> {
    let item_id = ctx.item_id;
    let full_text = ctx.full_text.clone();
    let base_ref = ctx.base_ref.clone();
    let force = ctx.force;
    run_db(&ctx.data_dir, move |conn| {
        create_summaries_inner(conn, item_id, &full_text, &base_ref, force)
    })
    .await
}

/// R15.4: one batched read of `(chunk_id, content)` per section (replaces the
/// per-section and per-tag chunk queries in summaries/tags).
fn load_section_chunks(
    conn: &Connection,
    sec_ids: &[i64],
) -> Result<std::collections::HashMap<i64, Vec<(i64, String)>>, String> {
    let mut out: std::collections::HashMap<i64, Vec<(i64, String)>> = std::collections::HashMap::new();
    if sec_ids.is_empty() {
        return Ok(out);
    }
    let sql = format!(
        "SELECT section_id, id, content FROM chunks WHERE section_id IN ({}) ORDER BY id",
        crate::common::placeholders(sec_ids.len())
    );
    let mut cstmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = cstmt
        .query_map(rusqlite::params_from_iter(sec_ids.iter()), |r| {
            Ok((r.get::<_, Option<i64>>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?))
        })
        .map_err(|e| e.to_string())?;
    for row in rows.filter_map(|r| r.ok()) {
        if let (Some(sid), cid, content) = row {
            out.entry(sid).or_default().push((cid, content));
        }
    }
    Ok(out)
}

fn create_summaries_inner(
    conn: &Connection,
    item_id: i64,
    full_text: &str,
    base_ref: &str,
    force: bool,
) -> Result<(), String> {
    let mut stmt = conn
        .prepare("SELECT id, title, path, level, summary_hash, summary FROM sections WHERE item_id=?1 ORDER BY id")
        .map_err(|e| e.to_string())?;
    let secs: Vec<(i64, String, String, i64, String, String)> = stmt
        .query_map([item_id], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();
    // R15.4: one batched read for all section texts (was one query per section).
    let sec_ids: Vec<i64> = secs.iter().map(|(id, _, _, _, _, _)| *id).collect();
    let mut section_chunks = load_section_chunks(conn, &sec_ids)?;
    for (sec_id, title, path, level, stored_hash, stored_summary) in secs {
        if title.is_empty() && path.is_empty() && level == 0 {
            continue;
        }
        let sec_text: String = section_chunks
            .remove(&sec_id)
            .unwrap_or_default()
            .into_iter()
            .map(|(_, c)| c)
            .collect::<Vec<_>>()
            .join("\n\n");
        if sec_text.trim().is_empty() {
            continue;
        }
        let new_hash = window_hash(&sec_text);
        if !force && stored_hash == new_hash && !stored_summary.is_empty() {
            continue;
        }
        let summary = extractive_section_summary(&sec_text);
        if summary.is_empty() {
            continue;
        }
        conn.execute(
            "UPDATE sections SET summary=?1, summary_hash=?2 WHERE id=?3",
            rusqlite::params![summary, new_hash, sec_id],
        )
        .map_err(|e| e.to_string())?;
        conn.execute(
            "DELETE FROM chunks WHERE section_id=?1 AND kind='section_summary'",
            [sec_id],
        )
        .map_err(|e| e.to_string())?;
        let src_ref = if title.is_empty() {
            format!("{} summary", base_ref)
        } else {
            format!("{} summary: {}", base_ref, title)
        };
        conn.execute(
            "INSERT INTO chunks (item_id, section_id, kind, source_ref, content, level, path, created_at, uid) VALUES (?1,?2,'section_summary',?3,?4,?5,?6,datetime('now'),?7)",
            rusqlite::params![item_id, sec_id, src_ref, summary, level, path, crate::db::new_uid()],
        )
        .map_err(|e| e.to_string())?;
    }
    // doc summary
    if full_text.trim().is_empty() {
        return Ok(());
    }
    let doc_summary = extractive_doc_summary(full_text);
    if doc_summary.is_empty() {
        return Ok(());
    }
    let new_doc_hash = window_hash(&doc_summary);
    let existing_doc: Option<String> = conn
        .query_row(
            "SELECT content FROM chunks WHERE item_id=?1 AND kind='doc_summary' LIMIT 1",
            [item_id],
            |r| r.get::<_, String>(0),
        )
        .ok();
    if !force {
        if let Some(existing) = existing_doc {
            if window_hash(&existing) == new_doc_hash {
                return Ok(());
            }
        }
    }
    conn.execute(
        "DELETE FROM chunks WHERE item_id=?1 AND kind='doc_summary'",
        [item_id],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO chunks (item_id, kind, source_ref, content, level, path, created_at, uid) VALUES (?1,'doc_summary',?2,?3,0,'',datetime('now'),?4)",
        rusqlite::params![item_id, format!("{} summary", base_ref), doc_summary, crate::db::new_uid()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

async fn create_tags(ctx: &ClassifyCtx) -> Result<(), String> {
    let item_id = ctx.item_id;
    let full_text = ctx.full_text.clone();
    run_db(&ctx.data_dir, move |conn| create_tags_inner(conn, item_id, &full_text)).await
}

fn create_tags_inner(conn: &Connection, item_id: i64, full_text: &str) -> Result<(), String> {
    use std::sync::atomic::Ordering;
    let t_all = std::time::Instant::now();
    let calls0 = crate::tags::ENSURE_CALLS.load(Ordering::Relaxed);
    let exact0 = crate::tags::ENSURE_EXACT.load(Ordering::Relaxed);
    let scans0 = crate::tags::ENSURE_SCANS.load(Ordering::Relaxed);
    let micros0 = crate::tags::ENSURE_MICROS.load(Ordering::Relaxed);
    let mut stmt = conn
        .prepare("SELECT id, title, path FROM sections WHERE item_id=?1 ORDER BY id")
        .map_err(|e| e.to_string())?;
    let secs: Vec<(i64, String, String)> = stmt
        .query_map([item_id], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();
    // R15.4: one batched read for all section chunks (was per-section content
    // + per-tag id queries).
    let sec_ids: Vec<i64> = secs.iter().map(|(id, _, _)| *id).collect();
    let mut section_chunks = load_section_chunks(conn, &sec_ids)?;
    for (sec_id, title, _path) in secs {
        if title.is_empty() {
            continue;
        }
        let chunks = section_chunks.remove(&sec_id).unwrap_or_default();
        let mut sec_text: String = chunks.iter().map(|(_, c)| c.clone()).collect::<Vec<_>>().join("\n\n");
        if sec_text.trim().is_empty() {
            sec_text = title.clone();
        }
        let chunk_ids: Vec<i64> = chunks.into_iter().map(|(cid, _)| cid).collect();
        let proposed = crate::tags::propose_tags(&sec_text, &title);
        for tag_name in proposed {
            let tag_id = crate::tags::ensure_tag(conn, &tag_name)?;
            for cid in &chunk_ids {
                let _ = conn.execute(
                    "INSERT OR IGNORE INTO chunk_tags (chunk_id, tag_id) VALUES (?1,?2)",
                    rusqlite::params![cid, tag_id],
                );
            }
        }
    }
    // also tag doc-level chunks (doc_summary) with top tags from full_text
    let doc_tags = crate::tags::propose_tags(full_text, "");
    if !doc_tags.is_empty() {
        // R15.4: hoisted out of the per-tag loop (was re-queried per tag).
        let doc_chunks: Vec<i64> = conn
            .prepare("SELECT id FROM chunks WHERE item_id=?1 AND kind='doc_summary'")
            .map_err(|e| e.to_string())?
            .query_map([item_id], |r| r.get::<_, i64>(0))
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();
        for tag_name in doc_tags.iter().take(2) {
            let tag_id = crate::tags::ensure_tag(conn, tag_name)?;
            for cid in &doc_chunks {
                let _ = conn.execute(
                    "INSERT OR IGNORE INTO chunk_tags (chunk_id, tag_id) VALUES (?1,?2)",
                    rusqlite::params![cid, tag_id],
                );
            }
        }
    }
    tracing::debug!(
        target: "anchorcore::perf",
        item = item_id,
        total_ms = t_all.elapsed().as_millis(),
        calls = crate::tags::ENSURE_CALLS.load(Ordering::Relaxed) - calls0,
        exact = crate::tags::ENSURE_EXACT.load(Ordering::Relaxed) - exact0,
        scans = crate::tags::ENSURE_SCANS.load(Ordering::Relaxed) - scans0,
        ensure_ms = (crate::tags::ENSURE_MICROS.load(Ordering::Relaxed) - micros0) / 1000,
        "tags"
    );
    Ok(())
}

async fn delete_stale_entities(ctx: &ClassifyCtx, window_index: Option<i64>) -> Result<(), String> {
    let item_id = ctx.item_id;
    run_db(&ctx.data_dir, move |conn| {
        let ids: Vec<i64> = match window_index {
            Some(index) => conn.prepare("SELECT id FROM entities WHERE item_id=?1 AND window_index=?2").unwrap().query_map(rusqlite::params![item_id, index], |r| r.get(0)).unwrap().filter_map(|r| r.ok()).collect(),
            None => conn.prepare("SELECT id FROM entities WHERE item_id=?1").unwrap().query_map([item_id], |r| r.get(0)).unwrap().filter_map(|r| r.ok()).collect(),
        };
        for eid in ids {
            let _ = conn.execute("DELETE FROM relationships WHERE from_entity_id=?1 OR to_entity_id=?1", [eid]);
            let _ = conn.execute("DELETE FROM merge_actions WHERE entity_a_id=?1 OR entity_b_id=?1", [eid]);
        }
        match window_index {
            Some(index) => {
                let _ = conn.execute("DELETE FROM entities WHERE item_id=?1 AND window_index=?2", rusqlite::params![item_id, index]);
            }
            None => {
                let _ = conn.execute("DELETE FROM entities WHERE item_id=?1", [item_id]);
            }
        }
        Ok(())
    }).await
}

fn classify_concurrency() -> usize {
    std::env::var("ANCHOR_CLASSIFIER_CONCURRENCY").ok().and_then(|v| v.parse().ok()).filter(|n| *n > 0).unwrap_or(4)
}

/// Bounded-concurrency LLM classification for all changed windows; results keep
/// window order. Batches keep cancellation latency at one model call.
/// `Classifier::classify` falls back to type-aware rules on failure.
async fn classify_all(
    classifier: &Arc<Classifier>,
    jobs: &[(i64, String, String)],
    doc_type: &str,
    trusted: bool,
    data_dir: &str,
    job_id: i64,
) -> Result<Vec<Vec<crate::classifier::ClassifiedItem>>, String> {
    let mut out: Vec<Vec<crate::classifier::ClassifiedItem>> = vec![Vec::new(); jobs.len()];
    let mut start = 0usize;
    for batch in jobs.chunks(classify_concurrency()) {
        if job_cancelled(data_dir, job_id).await? {
            return Err("job cancelled".to_string());
        }
        let mut set = tokio::task::JoinSet::new();
        for (offset, (_index, source_ref, window)) in batch.iter().enumerate() {
            let classifier = classifier.clone();
            let doc_type = doc_type.to_string();
            let source_ref = source_ref.clone();
            let window = window.clone();
            set.spawn(async move {
                let items = classifier.classify(&window, &source_ref, &doc_type, trusted).await;
                (offset, items)
            });
        }
        while let Some(joined) = set.join_next().await {
            match joined {
                Ok((offset, items)) => out[start + offset] = items,
                Err(e) => tracing::warn!("classification task failed: {}", e),
            }
        }
        start += batch.len();
    }
    Ok(out)
}

/// Attach the source window + index and dedupe by summary (per window).
fn prepare_items(items: Vec<crate::classifier::ClassifiedItem>, window: &str, source_ref: &str, index: i64) -> Vec<crate::classifier::ClassifiedItem> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for mut it in items {
        let key = it.summary.to_lowercase();
        if !seen.insert(key) {
            continue;
        }
        it.window_text = window.to_string();
        it.window_index = Some(index);
        if it.source_ref.is_empty() {
            it.source_ref = source_ref.to_string();
        }
        out.push(it);
    }
    out
}

async fn store_batch(ctx: &ClassifyCtx, items: Vec<crate::classifier::ClassifiedItem>) -> Result<usize, String> {
    let item_id = ctx.item_id;
    let count = items.len();
    run_db(&ctx.data_dir, move |conn| {
        for it in items {
            let status = if it.confidence >= 0.7 { "verified" } else { "unverified" };
            conn.execute("INSERT INTO entities (item_id, kind, summary, reasoning, confidence, author, source_ref, window_text, window_index, status, owner, created_at, updated_at, uid) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,'',datetime('now'),datetime('now'),?11)", rusqlite::params![item_id, it.kind, it.summary, it.reasoning, it.confidence, it.author, it.source_ref, it.window_text, it.window_index, status, crate::db::new_uid()]).unwrap();
            let eid = conn.last_insert_rowid();
            for chunk_content in crate::chunking::chunk_text(&it.summary) {
                let _ = conn.execute("INSERT INTO chunks (entity_id, kind, source_ref, content, created_at, uid) VALUES (?1,'entity',?2,?3,datetime('now'),?4)", rusqlite::params![eid, it.source_ref, chunk_content, crate::db::new_uid()]);
            }
        }
        Ok(())
    }).await?;
    Ok(count)
}

async fn update_window_hashes(ctx: &ClassifyCtx) -> Result<(), String> {
    let item_id = ctx.item_id;
    let hashes = serde_json::to_string(&ctx.new_hashes).unwrap();
    run_db(&ctx.data_dir, move |conn| {
        let _ = conn.execute("UPDATE ingested_items SET window_hashes=?1 WHERE id=?2", rusqlite::params![hashes, item_id]);
        Ok(())
    }).await
}

// full-document chunks + hierarchical sections (R14.1)
async fn store_doc_chunks(ctx: &ClassifyCtx) -> Result<(), String> {
    let doc_type = ctx.doc_type.clone();
    // symbol families dispatch on the filename; opaque external ids (Drive)
    // degrade to root-only sections, never to an error
    let filename = ctx.external_id.clone();
    let item_id = ctx.item_id;
    let full_text = ctx.full_text.clone();
    let base_ref = ctx.base_ref.clone();
    // B12 cleaning is prose-only — code/config splitters read indentation
    // (Python nesting, YAML levels), which whitespace collapsing destroys.
    let skip_clean = ctx.skips_extraction();
    run_db(&ctx.data_dir, move |conn| {
        let _ = conn.execute(
            "DELETE FROM chunks WHERE item_id=?1 AND entity_id IS NULL AND kind != 'distilled'",
            [item_id],
        );
        let _ = conn.execute("DELETE FROM sections WHERE item_id=?1", [item_id]);
        // B12 parity: clean BEFORE chunking so boundaries/embeddings/FTS
        // tokens are clean (the stored item text stays raw, like archived).
        let cleaned = if skip_clean {
            full_text.clone()
        } else {
            crate::chunking::clean_doc_text(&full_text)
        };
        let (metas, chunk_metas) = crate::chunking::chunk_for_doc_type(&doc_type, &filename, &cleaned);
        // insert sections in order so parent idx maps correctly
        let mut section_ids: Vec<i64> = Vec::with_capacity(metas.len());
        for meta in &metas {
            let parent_db = meta.parent.and_then(|p| section_ids.get(p).copied());
            conn.execute(
                "INSERT INTO sections (item_id, parent_id, level, title, path, created_at, uid) VALUES (?1,?2,?3,?4,?5,datetime('now'),?6)",
                rusqlite::params![item_id, parent_db, meta.level, meta.title, meta.path, crate::db::new_uid()],
            )
            .map_err(|e| e.to_string())?;
            section_ids.push(conn.last_insert_rowid());
        }
        // skip empty-title root sections that had no content? we kept all, but if root is empty with path="" it will have been inserted;
        // chunks from that root will still point to it — keep for completeness. Alternative: skip inserts where title empty and no chunks map.
        for (idx, cm) in chunk_metas.iter().enumerate() {
            let sec_db = cm.section_idx.and_then(|s| section_ids.get(s).copied());
            let r = if chunk_metas.len() > 1 {
                format!("{} §{}", base_ref, idx + 1)
            } else {
                base_ref.clone()
            };
            conn.execute(
                "INSERT INTO chunks (item_id, kind, source_ref, content, section_id, level, path, created_at, uid) VALUES (?1,'document',?2,?3,?4,?5,?6,datetime('now'),?7)",
                rusqlite::params![item_id, r, cm.content, sec_db, cm.level, cm.path, crate::db::new_uid()],
            )
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    })
    .await
}

// hash skip check: if all window hashes equal prev and not force, skip entirely.
// R15.4: compares already-computed `ClassifyCtx.new_hashes` (was re-hashing
// every window even though `classify_windows` had just hashed them).
fn distill_changed_hashes(force: bool, new_hashes: &std::collections::HashMap<String, String>, prev: &std::collections::HashMap<String, String>) -> bool {
    if force {
        return true;
    }
    for (k, h) in new_hashes {
        if prev.get(k).map(|v| v != h).unwrap_or(true) {
            return true;
        }
    }
    false
}

fn distill_unit_content(unit: &crate::classifier::DistilledUnit) -> String {
    let mut content = format!("Q: {}\nA: {}", unit.question, unit.answer);
    if !unit.terms.is_empty() {
        content.push_str(&format!("\nTerms: {}", unit.terms.join(", ")));
    }
    if !unit.systems.is_empty() {
        content.push_str(&format!("\nSystems: {}", unit.systems.join(", ")));
    }
    content
}

// B18 distillation (chat-like doc types → Q&A units), hash-skipped.
// R22.1: uses the LLM distiller (rules fallback inside `Classifier::distill`).
async fn distill_item(ctx: &ClassifyCtx) -> Result<(), String> {
    let distill_enabled = ctx.settings.get("distill_enabled", None).map(|v| v == "true" || v == "1" || v == "True").unwrap_or(true);
    // R33.4: structured files skip distillation too — paraphrase chunks are
    // the same noise class as extracted entities (and burn an LLM call).
    if ctx.skips_extraction()
        || !(crate::distill::DISTILL_DOC_TYPES.contains(&ctx.doc_type.as_str()) && distill_enabled)
    {
        // still need to update distill_hashes to current window hashes to avoid re-distilling later if type changes
        return update_distill_hashes(ctx).await;
    }
    let item_id = ctx.item_id;
    let force = ctx.force;
    // R15.4: reuse classify-phase hashes (no re-hash of every window).
    let new_hashes = ctx.new_hashes.clone();
    let prev: std::collections::HashMap<String, String> = run_db(&ctx.data_dir, move |conn| {
        let prev_str: String = conn.query_row("SELECT distill_hashes FROM ingested_items WHERE id=?1", [item_id], |r| r.get(0)).unwrap_or_else(|_| "{}".to_string());
        Ok(serde_json::from_str(&prev_str).unwrap_or_default())
    }).await?;
    if !distill_changed_hashes(force, &new_hashes, &prev) {
        return Ok(());
    }
    let jobs: Vec<(i64, String, String)> = ctx.windows.iter().enumerate().map(|(i, w)| {
        let index = (i + 1) as i64;
        let source_ref = if ctx.windows.len() > 1 { format!("{} §{}", ctx.base_ref, index) } else { ctx.base_ref.clone() };
        (index, source_ref, w.clone())
    }).collect();
    let units = distill_all(&ctx.classifier, &jobs, ctx.classify_trusted, &ctx.data_dir, ctx.job_id).await?;
    let max_units: usize = std::env::var("ANCHOR_DISTILL_MAX_UNITS").ok().and_then(|v| v.parse().ok()).unwrap_or(8);
    let item_id = ctx.item_id;
    run_db(&ctx.data_dir, move |conn| {
        // delete old distilled chunks
        let _ = conn.execute("DELETE FROM chunks WHERE item_id=?1 AND kind='distilled'", [item_id]);
        for ((_index, source_ref, _window), batch) in jobs.iter().zip(units) {
            for unit in batch.into_iter().take(max_units) {
                let content = distill_unit_content(&unit);
                let _ = conn.execute("INSERT INTO chunks (item_id, kind, source_ref, content, created_at, uid) VALUES (?1,'distilled',?2,?3,datetime('now'),?4)", rusqlite::params![item_id, source_ref, content, crate::db::new_uid()]);
            }
        }
        let j = serde_json::to_string(&new_hashes).unwrap_or_else(|_| "{}".to_string());
        let _ = conn.execute("UPDATE ingested_items SET distill_hashes=?1 WHERE id=?2", rusqlite::params![j, item_id]);
        Ok(())
    }).await
}

/// Bounded-concurrency LLM distillation for all windows; results keep window
/// order. Batches keep cancellation latency at one model call.
async fn distill_all(
    classifier: &Arc<Classifier>,
    jobs: &[(i64, String, String)],
    trusted: bool,
    data_dir: &str,
    job_id: i64,
) -> Result<Vec<Vec<crate::classifier::DistilledUnit>>, String> {
    let mut out: Vec<Vec<crate::classifier::DistilledUnit>> = vec![Vec::new(); jobs.len()];
    let mut start = 0usize;
    for batch in jobs.chunks(classify_concurrency()) {
        if job_cancelled(data_dir, job_id).await? {
            return Err("job cancelled".to_string());
        }
        let mut set = tokio::task::JoinSet::new();
        for (offset, (_index, source_ref, window)) in batch.iter().enumerate() {
            let classifier = classifier.clone();
            let source_ref = source_ref.clone();
            let window = window.clone();
            set.spawn(async move {
                let units = classifier.distill(&window, &source_ref, trusted).await;
                (offset, units)
            });
        }
        while let Some(joined) = set.join_next().await {
            match joined {
                Ok((offset, units)) => out[start + offset] = units,
                Err(e) => tracing::warn!("distillation task failed: {}", e),
            }
        }
        start += batch.len();
    }
    Ok(out)
}

async fn update_distill_hashes(ctx: &ClassifyCtx) -> Result<(), String> {
    let item_id = ctx.item_id;
    // R15.4: classify phase already hashed every window into `new_hashes`.
    let new_hashes = ctx.new_hashes.clone();
    run_db(&ctx.data_dir, move |conn| {
        if let Ok(j) = serde_json::to_string(&new_hashes) {
            let _ = conn.execute("UPDATE ingested_items SET distill_hashes=?1 WHERE id=?2", rusqlite::params![j, item_id]);
        }
        Ok(())
    }).await
}

// embed (gated) — wire embedder.embed_gated + sync_vec (P0 3.2)
/// One batch of chunks of `source_id` that still need a vector (ascending ids
/// so iteration always advances, even when the IDF gate embeds nothing).
async fn fetch_embed_batch(data_dir: &str, source_id: i64, last_id: i64) -> Result<Vec<(i64, String)>, String> {
    run_db(data_dir, move |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT id, content FROM chunks \
                 WHERE id > ?1 AND embedding IS NULL AND ( \
                   item_id IN (SELECT id FROM ingested_items WHERE source_id = ?2) \
                   OR entity_id IN (SELECT e.id FROM entities e JOIN ingested_items i ON i.id = e.item_id WHERE i.source_id = ?2)) \
                 ORDER BY id LIMIT 64",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(rusqlite::params![last_id, source_id], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(|e| e.to_string())?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    })
    .await
}

/// Write embeddings + keep the vec0 index in sync (R25.3 re-embed job).
async fn store_embeddings(data_dir: &str, embedder: Arc<Embedder>, updates: Vec<(i64, Vec<u8>)>) -> Result<(), String> {
    run_db(data_dir, move |conn| {
        for (cid, blob) in &updates {
            conn.execute("UPDATE chunks SET embedding=?1 WHERE id=?2", rusqlite::params![blob, cid])
                .map_err(|e| e.to_string())?;
            let _ = embedder.sync_vec(conn, *cid, blob);
        }
        Ok(())
    })
    .await
}

async fn embed_chunks(ctx: &ClassifyCtx) -> Result<(), String> {
    let embed_min_signal = embed_min_signal();
    let gated = sensitive_label(&ctx.label) && !(ctx.classify_trusted && ctx.embed_trusted);
    let item_id = ctx.item_id;
    let chunks: Vec<(i64, String)> = run_db(&ctx.data_dir, move |conn| {
        let mut stmt = conn.prepare("SELECT id, content FROM chunks WHERE (item_id=?1 OR entity_id IN (SELECT id FROM entities WHERE item_id=?1)) AND embedding IS NULL").unwrap();
        let rows = stmt.query_map([item_id], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))).unwrap().filter_map(|r| r.ok()).collect();
        Ok(rows)
    }).await?;
    if chunks.is_empty() {
        return Ok(());
    }
    let pending_ids: Vec<i64> = chunks.iter().map(|(id, _)| *id).collect();
    let contents: Vec<String> = chunks.iter().map(|(_, c)| c.clone()).collect();
    // call embedder (graceful fallback inside embed_gated on remote failure)
    let out = ctx.embedder.embed_gated(contents, gated, embed_min_signal).await.unwrap_or_else(|_| vec![None; pending_ids.len()]);
    let updates: Vec<(i64, Vec<u8>)> = out
        .into_iter()
        .enumerate()
        .filter_map(|(idx, blob_opt)| blob_opt.map(|blob| (pending_ids[idx], blob)))
        .collect();
    if updates.is_empty() {
        return Ok(());
    }
    // R15.4: single blocking task for all writes (was one spawn_blocking per chunk).
    let embedder = ctx.embedder.clone();
    run_db(&ctx.data_dir, move |conn| {
        for (cid, blob) in &updates {
            let _ = conn.execute("UPDATE chunks SET embedding=?1 WHERE id=?2", rusqlite::params![blob, cid]);
            let _ = embedder.sync_vec(conn, *cid, blob);
        }
        Ok(())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn extractive_helpers() {
        let sec = "para1 line1\n\npara2 line2\n\npara3 line3";
        let s = extractive_section_summary(sec);
        assert!(s.contains("para1"));
        assert!(s.contains("para2"));
        assert!(!s.contains("para3") || s.len() <= 1200);
        let doc = "doc intro\n\nmore";
        let d = extractive_doc_summary(doc);
        assert!(!d.is_empty());
    }

    #[test]
    fn creates_section_summaries() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.db");
        let conn = crate::db::init_db(&path).unwrap();
        conn.execute("INSERT INTO sources (name, connector, config, enabled, last_sync_cursor, error_count) VALUES ('s','folder','{}',1,'',0)", []).unwrap();
        let sid = conn.last_insert_rowid();
        let full_text = "§434 Refund\ncontent a refund details\n\n§437 Dates\ncontent b dates details\n\n4.2.1 Sub\ncontent c sub details";
        let hash = crate::hashing::content_hash(full_text);
        conn.execute("INSERT INTO ingested_items (source_id, external_id, title, text, content_hash, author, stale) VALUES (?1,?2,'doc',?3,?4,'',0)", rusqlite::params![sid, "ext1", full_text, hash]).unwrap();
        let iid = conn.last_insert_rowid();
        // simulate store_doc_chunks
        let (metas, chunk_metas) = crate::chunking::chunk_document_with_sections(full_text);
        let mut sids = Vec::new();
        for m in &metas {
            let parent = m.parent.and_then(|p| sids.get(p).copied());
            conn.execute("INSERT INTO sections (item_id, parent_id, level, title, path) VALUES (?1,?2,?3,?4,?5)", rusqlite::params![iid, parent, m.level, m.title, m.path]).unwrap();
            sids.push(conn.last_insert_rowid());
        }
        for (idx, cm) in chunk_metas.iter().enumerate() {
            let sec = cm.section_idx.and_then(|s| sids.get(s).copied());
            conn.execute("INSERT INTO chunks (item_id, section_id, kind, source_ref, content, level, path) VALUES (?1,?2,'document',?3,?4,?5,?6)", rusqlite::params![iid, sec, format!("doc §{}", idx+1), cm.content, cm.level, cm.path]).unwrap();
        }
        let base = "doc";
        create_summaries_inner(&conn, iid, full_text, base, false).unwrap();
        let cnt: i64 = conn.query_row("SELECT COUNT(*) FROM chunks WHERE kind='section_summary'", [], |r| r.get(0)).unwrap();
        assert_eq!(cnt, 3, "3 sections -> 3 section_summary");
        let doc_cnt: i64 = conn.query_row("SELECT COUNT(*) FROM chunks WHERE kind='doc_summary'", [], |r| r.get(0)).unwrap();
        assert_eq!(doc_cnt, 1);
        // hash skip: second call should not create duplicates
        create_summaries_inner(&conn, iid, full_text, base, false).unwrap();
        let cnt2: i64 = conn.query_row("SELECT COUNT(*) FROM chunks WHERE kind='section_summary'", [], |r| r.get(0)).unwrap();
        assert_eq!(cnt2, 3);
    }

    #[test]
    fn finish_job_keeps_sync_totals() {
        let dir = tempdir().unwrap();
        let data_dir = dir.path().to_string_lossy().to_string();
        let path = dir.path().join("test.db");
        let conn = crate::db::init_db(&path).unwrap();
        conn.execute("INSERT INTO sources (name, connector, config, enabled, last_sync_cursor, error_count) VALUES ('s','folder','{}',1,'',0)", []).unwrap();
        let sid = conn.last_insert_rowid();
        conn.execute("INSERT INTO jobs (source_id, kind, status, total, processed, result, created_at) VALUES (?1,'sync','running',10,10,'{}',datetime('now'))", [sid]).unwrap();
        let job_id = conn.last_insert_rowid();
        let store = crate::secrets::SecretStore::new(dir.path().join("secrets.enc"));
        let settings = Arc::new(SettingsService::new(store));
        let classifier = Arc::new(Classifier::new(settings.clone()));
        let embedder = Arc::new(Embedder::new(settings.clone()));
        let pipeline = Pipeline::new(classifier, embedder, settings, data_dir, crate::jobs::JobManager::new());
        pipeline.finish_job(&conn, sid, job_id, Ok((3, 5, 0, 7)));
        let (status, total, processed, result): (String, i64, i64, String) = conn
            .query_row("SELECT status, total, processed, result FROM jobs WHERE id=?1", [job_id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })
            .unwrap();
        assert_eq!(status, "done");
        assert_eq!((total, processed), (10, 10), "changed items must not clobber scanned totals");
        assert!(result.contains("\"items\":3"), "result: {}", result);
        assert!(result.contains("\"unchanged\":7"), "result: {}", result);
    }

    #[test]
    fn connector_supports_purge_matrix() {
        for c in ["folder", "jira", "gdrive", "linear"] {
            assert!(connector_supports_purge(c), "{} must purge", c);
        }
        for c in ["rest", "slack", ""] {
            assert!(!connector_supports_purge(c), "{} must not purge", c);
        }
    }

    #[test]
    fn purge_missing_removes_item_and_cascades() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.db");
        let conn = crate::db::init_db(&path).unwrap();
        conn.execute("INSERT INTO sources (name, connector, config, enabled, last_sync_cursor, error_count) VALUES ('s','folder','{}',1,'',0)", []).unwrap();
        let sid = conn.last_insert_rowid();
        let mk_item = |ext: &str| -> i64 {
            conn.execute("INSERT INTO ingested_items (source_id, external_id, title, text, content_hash, author, stale) VALUES (?1,?2,'t','text','h','',0)", rusqlite::params![sid, ext]).unwrap();
            conn.last_insert_rowid()
        };
        let keep = mk_item("keep.md");
        let gone = mk_item("gone.md");
        conn.execute("INSERT INTO entities (item_id, kind, summary) VALUES (?1,'decision','gone entity')", [gone]).unwrap();
        let gone_e = conn.last_insert_rowid();
        conn.execute("INSERT INTO entities (item_id, kind, summary) VALUES (?1,'decision','keep entity')", [keep]).unwrap();
        let keep_e = conn.last_insert_rowid();
        let emb = vec![0u8; 768 * 4];
        conn.execute("INSERT INTO chunks (item_id, kind, source_ref, content, embedding) VALUES (?1,'document','gone.md','gone content',?2)", rusqlite::params![gone, emb]).unwrap();
        let gone_c = conn.last_insert_rowid();
        conn.execute("INSERT INTO chunks (entity_id, kind, source_ref, content) VALUES (?1,'entity','gone.md','gone entity chunk')", [gone_e]).unwrap();
        conn.execute("INSERT INTO chunks (item_id, kind, source_ref, content) VALUES (?1,'document','keep.md','keep content')", [keep]).unwrap();
        let keep_c = conn.last_insert_rowid();
        conn.execute("INSERT INTO sections (item_id, level, title, path) VALUES (?1,1,'S','S')", [gone]).unwrap();
        conn.execute("INSERT INTO relationships (from_entity_id, to_entity_id, kind) VALUES (?1,?1,'related')", [gone_e]).unwrap();
        conn.execute("INSERT INTO disputes (entity_id, reason, user) VALUES (?1,'r','u')", [gone_e]).unwrap();
        conn.execute("INSERT INTO merge_actions (entity_a_id, entity_b_id) VALUES (?1,?2)", rusqlite::params![gone_e, keep_e]).unwrap();
        conn.execute("INSERT INTO tags (name, count) VALUES ('solo',1)", []).unwrap();
        let solo = conn.last_insert_rowid();
        conn.execute("INSERT INTO tags (name, count) VALUES ('shared',2)", []).unwrap();
        let shared = conn.last_insert_rowid();
        conn.execute("INSERT INTO chunk_tags (chunk_id, tag_id) VALUES (?1,?2)", rusqlite::params![gone_c, solo]).unwrap();
        conn.execute("INSERT INTO chunk_tags (chunk_id, tag_id) VALUES (?1,?2)", rusqlite::params![gone_c, shared]).unwrap();
        conn.execute("INSERT INTO chunk_tags (chunk_id, tag_id) VALUES (?1,?2)", rusqlite::params![keep_c, shared]).unwrap();
        let vec_before: i64 = conn.query_row("SELECT COUNT(*) FROM vec_chunks WHERE rowid=?1", [gone_c], |r| r.get(0)).unwrap();
        assert_eq!(vec_before, 1);
        let fts_before: i64 = conn.query_row("SELECT COUNT(*) FROM chunks_fts WHERE rowid=?1", [gone_c], |r| r.get(0)).unwrap();
        assert_eq!(fts_before, 1);

        let fetched: std::collections::HashSet<String> = ["keep.md".to_string()].into_iter().collect();
        let n = purge_missing_inner(&conn, sid, &fetched).unwrap();
        assert_eq!(n, 1);
        let items: i64 = conn.query_row("SELECT COUNT(*) FROM ingested_items", [], |r| r.get(0)).unwrap();
        assert_eq!(items, 1);
        let gone_entities: i64 = conn.query_row("SELECT COUNT(*) FROM entities WHERE item_id=?1", [gone], |r| r.get(0)).unwrap();
        assert_eq!(gone_entities, 0);
        let gone_chunks: i64 = conn.query_row("SELECT COUNT(*) FROM chunks WHERE item_id=?1 OR entity_id=?2", rusqlite::params![gone, gone_e], |r| r.get(0)).unwrap();
        assert_eq!(gone_chunks, 0);
        let rels: i64 = conn.query_row("SELECT COUNT(*) FROM relationships", [], |r| r.get(0)).unwrap();
        assert_eq!(rels, 0);
        let merges: i64 = conn.query_row("SELECT COUNT(*) FROM merge_actions", [], |r| r.get(0)).unwrap();
        assert_eq!(merges, 0);
        let disputes: i64 = conn.query_row("SELECT COUNT(*) FROM disputes", [], |r| r.get(0)).unwrap();
        assert_eq!(disputes, 0);
        let secs: i64 = conn.query_row("SELECT COUNT(*) FROM sections WHERE item_id=?1", [gone], |r| r.get(0)).unwrap();
        assert_eq!(secs, 0);
        let vec_after: i64 = conn.query_row("SELECT COUNT(*) FROM vec_chunks WHERE rowid=?1", [gone_c], |r| r.get(0)).unwrap();
        assert_eq!(vec_after, 0);
        let fts_after: i64 = conn.query_row("SELECT COUNT(*) FROM chunks_fts WHERE rowid=?1", [gone_c], |r| r.get(0)).unwrap();
        assert_eq!(fts_after, 0);
        let solo_left: i64 = conn.query_row("SELECT COUNT(*) FROM tags WHERE id=?1", [solo], |r| r.get(0)).unwrap();
        assert_eq!(solo_left, 0);
        let shared_count: i64 = conn.query_row("SELECT count FROM tags WHERE id=?1", [shared], |r| r.get(0)).unwrap();
        assert_eq!(shared_count, 1);
        let keep_entities: i64 = conn.query_row("SELECT COUNT(*) FROM entities WHERE item_id=?1", [keep], |r| r.get(0)).unwrap();
        assert_eq!(keep_entities, 1);
        let keep_chunks: i64 = conn.query_row("SELECT COUNT(*) FROM chunks WHERE item_id=?1", [keep], |r| r.get(0)).unwrap();
        assert_eq!(keep_chunks, 1);
        let keep_e_left: i64 = conn.query_row("SELECT COUNT(*) FROM entities WHERE id=?1", [keep_e], |r| r.get(0)).unwrap();
        assert_eq!(keep_e_left, 1);
    }

    #[tokio::test]
    async fn store_doc_chunks_sections_code_by_symbols() {
        let dir = tempdir().unwrap();
        let data_dir = dir.path().to_string_lossy().to_string();
        let db_path = crate::db::resolve_db_path(&data_dir);
        let conn = crate::db::init_db(&db_path).unwrap();
        conn.execute("INSERT INTO sources (name, connector, config, enabled, last_sync_cursor, error_count) VALUES ('s','folder','{}',1,'',0)", []).unwrap();
        let sid = conn.last_insert_rowid();
        conn.execute("INSERT INTO ingested_items (source_id, external_id, title, text, content_hash, author, stale) VALUES (?1,'lib.rs','lib.rs','t','h','',0)", [sid]).unwrap();
        let iid = conn.last_insert_rowid();
        drop(conn);
        let store = crate::secrets::SecretStore::new(dir.path().join("secrets.enc"));
        let settings = Arc::new(crate::settings::SettingsService::new(store));
        settings.set_data_dir(dir.path());
        let classifier = Arc::new(crate::classifier::Classifier::new(settings.clone()));
        let embedder = Arc::new(crate::embedder::Embedder::new(settings.clone()));
        let doc = IngestionDoc {
            external_id: "lib.rs".to_string(),
            title: "lib.rs".to_string(),
            text: "pub fn alpha() {\n}\n\npub fn beta() {\n}\n".to_string(),
            author: String::new(),
            updated_at: None,
            source_ref: "lib.rs".to_string(),
        };
        let mut ctx = ClassifyCtx::new(&data_dir, sid, 0, &doc, false, settings, classifier, embedder);
        ctx.item_id = iid;
        ctx.doc_type = "code".to_string();
        store_doc_chunks(&ctx).await.unwrap();
        let conn = crate::db::init_db(&db_path).unwrap();
        let mut stmt = conn.prepare("SELECT title FROM sections WHERE item_id=?1 ORDER BY id").unwrap();
        let titles: Vec<String> = stmt.query_map([iid], |r| r.get(0)).unwrap().filter_map(|r| r.ok()).collect();
        assert!(titles.contains(&"fn alpha".to_string()), "titles: {titles:?}");
        assert!(titles.contains(&"fn beta".to_string()), "titles: {titles:?}");
        let sec_chunks: i64 = conn.query_row("SELECT COUNT(*) FROM chunks WHERE item_id=?1 AND section_id IS NOT NULL", [iid], |r| r.get(0)).unwrap();
        assert!(sec_chunks >= 2, "each symbol keeps its chunks");
    }

    #[tokio::test]
    async fn code_items_skip_extraction_and_purge_legacy() {
        // R33.2: a code file yields zero entities (even with decision-like
        // text), keeps symbol chunks, and a re-sync deletes legacy entities
        // plus their entity chunks. Runs degraded — no models needed.
        let dir = tempdir().unwrap();
        let data_dir = dir.path().to_string_lossy().to_string();
        let db_path = crate::db::resolve_db_path(&data_dir);
        let conn = crate::db::init_db(&db_path).unwrap();
        conn.execute("INSERT INTO sources (name, connector, config, enabled, last_sync_cursor, error_count) VALUES ('s','folder','{}',1,'',0)", []).unwrap();
        let sid = conn.last_insert_rowid();
        let text = "pub fn alpha() {\n    // we decided to use sqlite-vec for vectors\n}\n";
        let hash = crate::hashing::content_hash(text);
        conn.execute("INSERT INTO ingested_items (source_id, external_id, title, text, content_hash, author, stale) VALUES (?1,'lib.rs','lib.rs',?2,?3,'',0)", rusqlite::params![sid, text, hash]).unwrap();
        let iid = conn.last_insert_rowid();
        // legacy code entity + chunk from before R33.2
        conn.execute("INSERT INTO entities (item_id, kind, summary, source_ref, status) VALUES (?1,'decision','legacy code decision','lib.rs','unverified')", [iid]).unwrap();
        let eid = conn.last_insert_rowid();
        conn.execute("INSERT INTO chunks (entity_id, kind, source_ref, content) VALUES (?1,'entity','lib.rs','legacy code decision')", [eid]).unwrap();
        conn.execute("INSERT INTO jobs (source_id, kind, status, total, processed, result, created_at) VALUES (?1,'sync','running',1,0,'{}',datetime('now'))", [sid]).unwrap();
        let jid = conn.last_insert_rowid();
        drop(conn);
        let store = crate::secrets::SecretStore::new(dir.path().join("secrets.enc"));
        let settings = Arc::new(crate::settings::SettingsService::new(store));
        settings.set_data_dir(dir.path());
        let classifier = Arc::new(crate::classifier::Classifier::new(settings.clone()));
        let embedder = Arc::new(crate::embedder::Embedder::new(settings.clone()));
        let pipe = Pipeline::new_for_test(classifier, embedder, settings, data_dir.clone());
        let doc = IngestionDoc {
            external_id: "lib.rs".to_string(),
            title: "lib.rs".to_string(),
            text: text.to_string(),
            author: String::new(),
            updated_at: None,
            source_ref: "lib.rs".to_string(),
        };
        let n = pipe.classify_and_store(&data_dir, sid, jid, &doc, false).await.unwrap();
        assert_eq!(n, 0);
        let conn = crate::db::init_db(&db_path).unwrap();
        let entities: i64 = conn.query_row("SELECT COUNT(*) FROM entities WHERE item_id=?1", [iid], |r| r.get(0)).unwrap();
        assert_eq!(entities, 0, "legacy code entities purged on re-sync");
        let entity_chunks: i64 = conn.query_row("SELECT COUNT(*) FROM chunks WHERE kind='entity'", [], |r| r.get(0)).unwrap();
        assert_eq!(entity_chunks, 0, "legacy entity chunks cascade-purged");
        let chunks: i64 = conn.query_row("SELECT COUNT(*) FROM chunks WHERE item_id=?1 AND kind='document' AND section_id IS NOT NULL", [iid], |r| r.get(0)).unwrap();
        assert!(chunks >= 1, "symbol chunks still stored");
        let doc_type: String = conn.query_row("SELECT doc_type FROM ingested_items WHERE id=?1", [iid], |r| r.get(0)).unwrap();
        assert_eq!(doc_type, "code");
    }

    #[tokio::test]
    async fn config_files_skip_extraction_and_chunk_by_key() {
        // R33.4: a YAML file yields zero entities and key sections — same
        // harness as the code-skip test. The doc_type stays a detected prose
        // type (not asserted: live Ollama would answer the detect call).
        let dir = tempdir().unwrap();
        let data_dir = dir.path().to_string_lossy().to_string();
        let db_path = crate::db::resolve_db_path(&data_dir);
        let conn = crate::db::init_db(&db_path).unwrap();
        conn.execute("INSERT INTO sources (name, connector, config, enabled, last_sync_cursor, error_count) VALUES ('s','folder','{}',1,'',0)", []).unwrap();
        let sid = conn.last_insert_rowid();
        let text = "server:\n  host: example.com\n  tls:\n    cert_file: /etc/cert.pem\n";
        let hash = crate::hashing::content_hash(text);
        conn.execute("INSERT INTO ingested_items (source_id, external_id, title, text, content_hash, author, stale) VALUES (?1,'values.yaml','values.yaml',?2,?3,'',0)", rusqlite::params![sid, text, hash]).unwrap();
        let iid = conn.last_insert_rowid();
        conn.execute("INSERT INTO jobs (source_id, kind, status, total, processed, result, created_at) VALUES (?1,'sync','running',1,0,'{}',datetime('now'))", [sid]).unwrap();
        let jid = conn.last_insert_rowid();
        drop(conn);
        let store = crate::secrets::SecretStore::new(dir.path().join("secrets.enc"));
        let settings = Arc::new(crate::settings::SettingsService::new(store));
        settings.set_data_dir(dir.path());
        let classifier = Arc::new(crate::classifier::Classifier::new(settings.clone()));
        let embedder = Arc::new(crate::embedder::Embedder::new(settings.clone()));
        let pipe = Pipeline::new_for_test(classifier, embedder, settings, data_dir.clone());
        let doc = IngestionDoc {
            external_id: "values.yaml".to_string(),
            title: "values.yaml".to_string(),
            text: text.to_string(),
            author: String::new(),
            updated_at: None,
            source_ref: "values.yaml".to_string(),
        };
        let n = pipe.classify_and_store(&data_dir, sid, jid, &doc, false).await.unwrap();
        assert_eq!(n, 0);
        let conn = crate::db::init_db(&db_path).unwrap();
        let entities: i64 = conn.query_row("SELECT COUNT(*) FROM entities WHERE item_id=?1", [iid], |r| r.get(0)).unwrap();
        assert_eq!(entities, 0, "configs yield no entities");
        let distilled: i64 = conn.query_row("SELECT COUNT(*) FROM chunks WHERE item_id=?1 AND kind='distilled'", [iid], |r| r.get(0)).unwrap();
        assert_eq!(distilled, 0, "configs are not distilled");
        let mut stmt = conn.prepare("SELECT path FROM sections WHERE item_id=?1").unwrap();
        let paths: Vec<String> = stmt.query_map([iid], |r| r.get(0)).unwrap().filter_map(|r| r.ok()).collect();
        assert!(paths.iter().any(|p| p == "server > tls > cert_file"), "key breadcrumb stored: {paths:?}");
    }

    #[tokio::test]
    async fn purge_for_sync_guards_rest_and_empty() {
        let dir = tempdir().unwrap();
        let data_dir = dir.path().to_string_lossy().to_string();
        let db_path = crate::db::resolve_db_path(&data_dir);
        let conn = crate::db::init_db(&db_path).unwrap();
        conn.execute("INSERT INTO sources (name, connector, config, enabled, last_sync_cursor, error_count) VALUES ('s','folder','{}',1,'',0)", []).unwrap();
        let sid = conn.last_insert_rowid();
        conn.execute("INSERT INTO ingested_items (source_id, external_id, title, text, content_hash, author, stale) VALUES (?1,'a.md','t','text','h','',0)", [sid]).unwrap();
        drop(conn);
        let doc = IngestionDoc {
            external_id: "b.md".to_string(),
            title: "b".to_string(),
            text: "x".to_string(),
            author: String::new(),
            updated_at: None,
            source_ref: String::new(),
        };
        // rest connector never purges, even when the item is absent
        let n = purge_missing_for_sync(&data_dir, sid, "rest", std::slice::from_ref(&doc), &[]).await;
        assert_eq!(n, 0);
        // empty fetch purges nothing
        let n = purge_missing_for_sync(&data_dir, sid, "folder", &[], &[]).await;
        assert_eq!(n, 0);
        // an unreadable-but-present file (skipped) is never purged
        let n = purge_missing_for_sync(&data_dir, sid, "folder", &[], &["a.md".to_string()]).await;
        assert_eq!(n, 0);
        let conn = crate::db::open_db(&db_path).unwrap();
        let items: i64 = conn.query_row("SELECT COUNT(*) FROM ingested_items", [], |r| r.get(0)).unwrap();
        assert_eq!(items, 1);
    }

    fn write_doc(dir: &std::path::Path, name: &str, content: &str) {
        std::fs::write(dir.join(name), content).unwrap();
    }

    /// Folder source + settings pinned to closed ports, so the classifier and
    /// embedder fail fast into their degraded paths (no network in tests).
    fn seed_folder_source(data_dir: &str, folder: &std::path::Path) -> (Arc<SettingsService>, i64) {
        let db_path = crate::db::resolve_db_path(data_dir);
        let conn = crate::db::init_db(&db_path).unwrap();
        let store = crate::secrets::SecretStore::new(std::path::PathBuf::from(data_dir).join("secrets.enc"));
        let settings = Arc::new(crate::settings::SettingsService::new(store));
        settings.set_data_dir(std::path::Path::new(data_dir));
        settings.set("classifier_base_url", "http://127.0.0.1:9", &conn).unwrap();
        settings.set("embed_base_url", "http://127.0.0.1:9", &conn).unwrap();
        let cfg = serde_json::json!({"path": folder.to_string_lossy()}).to_string();
        conn.execute(
            "INSERT INTO sources (name, connector, config, enabled, last_sync_cursor, error_count) VALUES ('f','folder',?1,1,'',0)",
            [cfg],
        )
        .unwrap();
        let sid = conn.last_insert_rowid();
        (settings, sid)
    }

    /// One folder sync on a fresh running job. Returns
    /// `(items, entities, purged, unchanged, job_id)`.
    async fn run_folder_once(
        pipe: &Pipeline,
        data_dir: &str,
        sid: i64,
        cfg: &serde_json::Value,
        force: bool,
    ) -> (i64, i64, i64, i64, i64) {
        let db_path = crate::db::resolve_db_path(data_dir);
        let conn = crate::db::init_db(&db_path).unwrap();
        conn.execute(
            "INSERT INTO jobs (source_id, kind, status, total, processed, result, created_at) VALUES (?1,'sync','running',0,0,'{}',datetime('now'))",
            [sid],
        )
        .unwrap();
        let jid = conn.last_insert_rowid();
        drop(conn);
        let (items, entities, purged, unchanged) = pipe.run_folder_sync(sid, jid, force, cfg, data_dir).await.unwrap();
        (items, entities, purged, unchanged, jid)
    }

    fn job_row(data_dir: &str, jid: i64) -> (i64, i64, serde_json::Value) {
        let db_path = crate::db::resolve_db_path(data_dir);
        let conn = crate::db::init_db(&db_path).unwrap();
        let (total, processed, result): (i64, i64, String) = conn
            .query_row("SELECT total, processed, result FROM jobs WHERE id=?1", [jid], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .unwrap();
        (total, processed, serde_json::from_str(&result).unwrap())
    }

    fn table_count(data_dir: &str, table: &str) -> i64 {
        let db_path = crate::db::resolve_db_path(data_dir);
        let conn = crate::db::init_db(&db_path).unwrap();
        conn.query_row(&format!("SELECT COUNT(*) FROM {}", table), [], |r| r.get(0)).unwrap()
    }

    #[tokio::test]
    async fn folder_sync_skips_unchanged_files() {
        let data_tmp = tempdir().unwrap();
        let folder_tmp = tempdir().unwrap();
        let data_dir = data_tmp.path().to_string_lossy().to_string();
        let folder = folder_tmp.path();
        write_doc(folder, "a.md", "# Alpha\n\nWe decided to use sqlite for vectors. Next step: review on Friday.\n");
        write_doc(folder, "b.md", "# Beta\n\nWe agreed to postpone the launch. Owner: Bea will follow up.\n");
        let (settings, sid) = seed_folder_source(&data_dir, folder);
        let classifier = Arc::new(crate::classifier::Classifier::new(settings.clone()));
        let embedder = Arc::new(crate::embedder::Embedder::new(settings.clone()));
        let pipe = Pipeline::new_for_test(classifier, embedder, settings, data_dir.clone());
        let cfg = serde_json::json!({"path": folder.to_string_lossy()});

        // Legacy row: a pre-migration item carries content but no fingerprint.
        let text_a = std::fs::read_to_string(folder.join("a.md")).unwrap();
        let db_path = crate::db::resolve_db_path(&data_dir);
        let conn = crate::db::init_db(&db_path).unwrap();
        conn.execute(
            "INSERT INTO ingested_items (source_id, external_id, title, text, content_hash) VALUES (?1,'a.md','a.md',?2,?3)",
            rusqlite::params![sid, text_a, content_hash(&text_a)],
        )
        .unwrap();
        drop(conn);

        // First sync ingests the new file; the legacy row is extracted once
        // (fingerprint unknown) but hash-identical, so it is not reclassified.
        // Progress totals count files scanned while `result` carries live counts.
        let (items, entities, purged, unchanged, jid) = run_folder_once(&pipe, &data_dir, sid, &cfg, false).await;
        assert_eq!((items, purged, unchanged), (1, 0, 0));
        assert!(entities >= 2, "rules must extract decision/action entities, got {entities}");
        let (total, processed, result) = job_row(&data_dir, jid);
        assert_eq!((total, processed), (2, 2));
        assert_eq!(result.get("items").and_then(|v| v.as_i64()), Some(1));
        assert_eq!(result.get("unchanged").and_then(|v| v.as_i64()), Some(0));
        let chunks_after_first = table_count(&data_dir, "chunks");
        assert!(chunks_after_first > 0);
        // Fingerprints are stored for both files (legacy row backfilled).
        let conn = crate::db::init_db(&db_path).unwrap();
        let fingerprinted: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM ingested_items WHERE source_id=?1 AND file_mtime_ms IS NOT NULL AND file_size IS NOT NULL",
                [sid],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(fingerprinted, 2);
        drop(conn);

        // No changes: both files skipped without extraction — no reclassify,
        // no new rows anywhere.
        let before_entities = table_count(&data_dir, "entities");
        let (items, entities, purged, unchanged, jid) = run_folder_once(&pipe, &data_dir, sid, &cfg, false).await;
        assert_eq!((items, entities, purged, unchanged), (0, 0, 0, 2));
        assert_eq!(table_count(&data_dir, "entities"), before_entities);
        assert_eq!(table_count(&data_dir, "chunks"), chunks_after_first);
        let (total, processed, result) = job_row(&data_dir, jid);
        assert_eq!((total, processed), (2, 2));
        assert_eq!(result.get("items").and_then(|v| v.as_i64()), Some(0));
        assert_eq!(result.get("unchanged").and_then(|v| v.as_i64()), Some(2));

        // One new file: only it is processed.
        write_doc(folder, "c.md", "# Gamma\n\nWe chose the blue design. Deadline is Monday.\n");
        let (items, _, purged, unchanged, _) = run_folder_once(&pipe, &data_dir, sid, &cfg, false).await;
        assert_eq!((items, purged, unchanged), (1, 0, 2));

        // One edited file: only it is reclassified.
        std::thread::sleep(std::time::Duration::from_millis(10));
        write_doc(folder, "b.md", "# Beta\n\nWe agreed to launch next week instead. Owner: Bea will follow up.\n");
        let (items, _, purged, unchanged, _) = run_folder_once(&pipe, &data_dir, sid, &cfg, false).await;
        assert_eq!((items, purged, unchanged), (1, 0, 2));

        // One deleted file: purged, nothing reclassified.
        std::fs::remove_file(folder.join("a.md")).unwrap();
        let (items, _, purged, unchanged, _) = run_folder_once(&pipe, &data_dir, sid, &cfg, false).await;
        assert_eq!((items, purged, unchanged), (0, 1, 2));
        let conn = crate::db::init_db(&db_path).unwrap();
        let gone: i64 = conn
            .query_row("SELECT COUNT(*) FROM ingested_items WHERE source_id=?1 AND external_id='a.md'", [sid], |r| r.get(0))
            .unwrap();
        assert_eq!(gone, 0);
        let kept: i64 = conn
            .query_row("SELECT COUNT(*) FROM ingested_items WHERE source_id=?1", [sid], |r| r.get(0))
            .unwrap();
        assert_eq!(kept, 2);
        drop(conn);

        // Touched-but-identical: extracted once, not reclassified — and the
        // fingerprint is refreshed so the next sync skips it outright. Only
        // `b.md` counts as unchanged (it was never extracted).
        std::thread::sleep(std::time::Duration::from_millis(10));
        write_doc(folder, "c.md", "# Gamma\n\nWe chose the blue design. Deadline is Monday.\n");
        let (items, _, purged, unchanged, _) = run_folder_once(&pipe, &data_dir, sid, &cfg, false).await;
        assert_eq!((items, purged, unchanged), (0, 0, 1));
        let conn = crate::db::init_db(&db_path).unwrap();
        let (mtime, size): (i64, i64) = conn
            .query_row(
                "SELECT file_mtime_ms, file_size FROM ingested_items WHERE source_id=?1 AND external_id='c.md'",
                [sid],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        drop(conn);
        let meta = std::fs::metadata(folder.join("c.md")).unwrap();
        let stat_mtime = meta.modified().unwrap().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64;
        assert_eq!((mtime, size), (stat_mtime, meta.len() as i64));

        // Force reclassifies everything, like the generic path.
        let (items, _, purged, unchanged, _) = run_folder_once(&pipe, &data_dir, sid, &cfg, true).await;
        assert_eq!((items, purged, unchanged), (2, 0, 0));
    }

    #[test]
    fn purge_missing_keeps_everything_when_all_fetched() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.db");
        let conn = crate::db::init_db(&path).unwrap();
        conn.execute("INSERT INTO sources (name, connector, config, enabled, last_sync_cursor, error_count) VALUES ('s','folder','{}',1,'',0)", []).unwrap();
        let sid = conn.last_insert_rowid();
        conn.execute("INSERT INTO ingested_items (source_id, external_id, title, text, content_hash, author, stale) VALUES (?1,'a.md','t','text','h','',0)", [sid]).unwrap();
        let fetched: std::collections::HashSet<String> = ["a.md".to_string()].into_iter().collect();
        let n = purge_missing_inner(&conn, sid, &fetched).unwrap();
        assert_eq!(n, 0);
        let items: i64 = conn.query_row("SELECT COUNT(*) FROM ingested_items", [], |r| r.get(0)).unwrap();
        assert_eq!(items, 1);
    }
}
