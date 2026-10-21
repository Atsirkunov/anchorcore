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
            // The stored config never holds the secret: create/update strips
            // it into the SecretStore, so "absent" is the normal case and
            // must be filled — not just "***set***" placeholders (those only
            // exist in GET /config output, never on disk). Any other value
            // is legacy plaintext and stays untouched.
            let needs_fill = matches!(obj.get(field).and_then(|x| x.as_str()), None | Some("***set***"));
            if needs_fill {
                if let Some(real) = store.get(&format!("source:{}:{}", source_id, field)) {
                    obj.insert(field.to_string(), serde_json::Value::String(real));
                }
            }
        }
        // linear uses api_key which is already in SECRET_SOURCE_FIELDS, but also handle "token" alias
        let api_key_missing = matches!(obj.get("api_key").and_then(|v| v.as_str()), None | Some("***set***"));
        if api_key_missing {
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
            let Some(last) = batch.last().map(|(id, _, _)| *id) else {
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
            let contents: Vec<String> = batch.iter().map(|(_, c, _)| c.clone()).collect();
            let embed_texts: Vec<String> = batch.iter().map(|(_, c, p)| crate::markdown::embed_input(p, c)).collect();
            let out = self
                .embedder
                .embed_gated(contents, embed_texts, gated, embed_min_signal())
                .await
                .unwrap_or_else(|_| vec![None; batch.len()]);
            let updates: Vec<(i64, Vec<u8>)> = batch
                .iter()
                .zip(out)
                .filter_map(|((id, _, _), blob)| blob.map(|b| (*id, b)))
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
            Box::pin(self.mark_item_processed(data_dir, source_id, &doc.external_id, &content_hash(&doc.text), None)).await?;
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
            Box::pin(self.mark_item_processed(data_dir, source_id, &doc.external_id, &content_hash(&doc.text), Some((f.mtime_ms, f.len)))).await?;
            return Ok(FileOutcome::Classified(n));
        }
        // Touched-but-identical: extracted, hash unchanged — processed, no counts.
        Ok(FileOutcome::NoChange)
    }

    async fn upsert_doc(&self, data_dir: &str, source_id: i64, doc: &IngestionDoc, _force: bool, fp: Option<(u64, u64)>) -> Result<bool, String> {
        let digest = content_hash(&doc.text);
        let doc = doc.clone();
        run_db(data_dir, move |conn| {
            let existing: Option<(i64, String, Option<String>, Option<String>)> = conn.query_row("SELECT id, content_hash, text, processed_hash FROM ingested_items WHERE source_id=?1 AND external_id=?2", rusqlite::params![source_id, doc.external_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).ok();
            if let Some((id, existing_hash, existing_text, processed)) = existing {
                let missing_text = existing_text.as_deref().unwrap_or("").is_empty();
                // F-03: "stored" (content_hash) and "fully processed"
                // (processed_hash) are separate markers — a cancel between
                // upsert and finalize must re-process, not skip. The stat
                // fingerprint is written only by mark_item_processed (and the
                // refresh below), never alongside unprocessed content.
                if existing_hash == digest && processed.as_deref() == Some(digest.as_str()) && !missing_text {
                    // Touched-but-identical: still refresh the stat fingerprint
                    // so the next sync skips extraction instead of retrying it.
                    store_fingerprint(conn, id, fp);
                    return Ok(false);
                }
                let _ = conn.execute("UPDATE ingested_items SET content_hash=?1, title=?2, author=?3, text=?4, stale=0 WHERE id=?5", rusqlite::params![digest, doc.title, doc.author, doc.text, id]);
                Ok(true)
            } else {
                conn.execute("INSERT INTO ingested_items (source_id, external_id, title, text, content_hash, author, stale, created_at, uid) VALUES (?1,?2,?3,?4,?5,?6, 0, datetime('now'),?7)", rusqlite::params![source_id, doc.external_id, doc.title, doc.text, digest, doc.author, crate::db::new_uid()]).map_err(|e| e.to_string())?;
                Ok(true)
            }
        }).await
    }

    /// F-03: completion marker, written only after a full successful
    /// classify + finalize. Carries the stat fingerprint with it, so a
    /// cancel/failure anywhere earlier leaves the item reprocessable:
    /// stale fingerprint → re-extract, stale processed_hash → reclassify.
    async fn mark_item_processed(&self, data_dir: &str, source_id: i64, external_id: &str, digest: &str, fp: Option<(u64, u64)>) -> Result<(), String> {
        let external_id = external_id.to_string();
        let digest = digest.to_string();
        run_db(data_dir, move |conn| {
            conn.execute("UPDATE ingested_items SET processed_hash=?1 WHERE source_id=?2 AND external_id=?3", rusqlite::params![digest, source_id, external_id]).map_err(|e| e.to_string())?;
            if let Some((mtime_ms, len)) = fp {
                conn.execute("UPDATE ingested_items SET file_mtime_ms=?1, file_size=?2 WHERE source_id=?3 AND external_id=?4", rusqlite::params![mtime_ms as i64, len as i64, source_id, external_id]).map_err(|e| e.to_string())?;
            }
            Ok(())
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

    // R33.6 deterministic bucket pre-pass: extension fast path first
    // (source files are `code` whatever the content reads like — comments
    // may read like prose), else prose markers → `record` vs `reference`
    // (`classifier::assign_bucket`). No LLM (the 8-way detect collapsed
    // 20/20 files to `documentation` — `docs/audit-r33.6-kinds.md` §1), no
    // trust gate, no boxing. Cached on the item; force reclassify
    // re-buckets (recovers legacy 8-way labels deterministically).
    async fn detect_doc_type(&mut self) -> Result<(), String> {
        self.classify_trusted = cloud_trusted(&*self.settings, "classifier");
        self.embed_trusted = cloud_trusted(&*self.settings, "embed");
        if !self.doc_type.is_empty() && !self.force {
            return Ok(());
        }
        let doc_type = if crate::extract::is_code_filename(&self.external_id)
            || crate::extract::is_code_filename(&self.base_ref)
        {
            "code".to_string()
        } else {
            crate::classifier::assign_bucket(&self.full_text)
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

    /// B26: hash-skip planning — which windows need (re)classification.
    /// Split out of `classify_windows` (gate hygiene: the R40.7 vocab hook
    /// pushed the orchestrator past CCN 15). Pure scan, no I/O.
    fn plan_window_jobs(&mut self) -> Vec<(i64, String, String)> {
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
        jobs
    }

    async fn classify_windows(&mut self) -> Result<usize, String> {
        // B26/R22.1: per-window LLM extraction (rules fallback inside
        // `Classifier::classify`), hash-skipped and bounded-concurrency.
        let jobs = self.plan_window_jobs();
        if jobs.is_empty() {
            return Ok(0);
        }
        if job_cancelled(&self.data_dir, self.job_id).await? {
            return Err("job cancelled".to_string());
        }
        let classified = classify_all(&self.classifier, &jobs, &self.doc_type, self.classify_trusted, &self.data_dir, self.job_id).await?;
        let mut total = 0usize;
        let mut llm_terms: Vec<crate::vocab::DefinedTerm> = Vec::new();
        for ((index, source_ref, window), output) in jobs.iter().zip(classified) {
            if job_cancelled(&self.data_dir, self.job_id).await? {
                return Err("job cancelled".to_string());
            }
            // delete stale entities for this window index if not force (already handled force)
            if !self.force {
                delete_stale_entities(self, Some(*index)).await?;
            }
            total += store_batch(self, prepare_items(output.items, window, source_ref, *index)).await?;
            for term in output.defined_terms {
                if !llm_terms.contains(&term) {
                    llm_terms.push(term);
                }
            }
        }
        self.store_vocab(llm_terms).await?;
        Ok(total)
    }

    /// R40.7: Tier-1 rules over the full text + Tier-2 union across windows,
    /// stored (proposed until corroborated). Skipped for structured files
    /// (code/config carry no vocabulary) and for hash-skipped items (the
    /// early return above leaves existing rows valid).
    async fn store_vocab(&self, llm_terms: Vec<crate::vocab::DefinedTerm>) -> Result<(), String> {
        if self.skips_extraction() {
            return Ok(());
        }
        if job_cancelled(&self.data_dir, self.job_id).await? {
            return Err("job cancelled".to_string());
        }
        let rules = crate::vocab::extract_vocab_rules(&self.full_text);
        if rules.is_empty() && llm_terms.is_empty() {
            return Ok(());
        }
        let item_id = self.item_id;
        let source_id = self.source_id;
        run_db(&self.data_dir, move |conn| {
            crate::vocab::store_vocab(conn, item_id, source_id, &rules, &llm_terms)
        })
        .await
    }

    /// F-08: prompt cooperative cancellation — every finalize phase
    /// re-checks, so a cancelled job stops starting new work (LLM batches,
    /// embedding) instead of running the whole chain to completion. An
    /// in-flight HTTP batch still completes (unsendable), but nothing new
    /// starts after the cancel lands.
    async fn check_cancelled(&self) -> Result<(), String> {
        if job_cancelled(&self.data_dir, self.job_id).await? {
            return Err("job cancelled".to_string());
        }
        Ok(())
    }

    async fn finalize(&self, perf: &mut PhaseTimes) -> Result<(), String> {
        self.check_cancelled().await?;
        update_window_hashes(self).await?;
        perf.mark("hashes");
        self.check_cancelled().await?;
        store_doc_chunks(self).await?;
        perf.mark("chunks");
        self.check_cancelled().await?;
        create_summaries(self).await?;
        perf.mark("summaries");
        self.check_cancelled().await?;
        create_tags(self).await?;
        perf.mark("tags");
        self.check_cancelled().await?;
        distill_item(self).await?;
        perf.mark("distill");
        self.check_cancelled().await?;
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
        .prepare("SELECT id, title, path, level, summary_hash, summary, line_start, line_end FROM sections WHERE item_id=?1 ORDER BY id")
        .map_err(|e| e.to_string())?;
    let secs: Vec<(i64, String, String, i64, String, String, i64, i64)> = stmt
        .query_map([item_id], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, i64>(6)?,
                r.get::<_, i64>(7)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();
    // R15.4: one batched read for all section texts (was one query per section).
    let sec_ids: Vec<i64> = secs.iter().map(|(id, _, _, _, _, _, _, _)| *id).collect();
    let mut section_chunks = load_section_chunks(conn, &sec_ids)?;
    for (sec_id, title, path, level, stored_hash, stored_summary, line_start, line_end) in secs {
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
            "INSERT INTO chunks (item_id, section_id, kind, source_ref, content, level, path, line_start, line_end, created_at, uid) VALUES (?1,?2,'section_summary',?3,?4,?5,?6,?7,?8,datetime('now'),?9)",
            rusqlite::params![item_id, sec_id, src_ref, summary, level, path, line_start, line_end, crate::db::new_uid()],
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
        "INSERT INTO chunks (item_id, kind, source_ref, content, level, path, line_start, line_end, created_at, uid) VALUES (?1,'doc_summary',?2,?3,0,'',1,?4,datetime('now'),?5)",
        rusqlite::params![item_id, format!("{} summary", base_ref), doc_summary, full_text.split('\n').count().max(1) as i64, crate::db::new_uid()],
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
) -> Result<Vec<crate::classifier::ClassifyOutput>, String> {
    let empty = || crate::classifier::ClassifyOutput { items: Vec::new(), defined_terms: Vec::new() };
    let mut out: Vec<crate::classifier::ClassifyOutput> = (0..jobs.len()).map(|_| empty()).collect();
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
                let output = classifier.classify(&window, &source_ref, &doc_type, trusted).await;
                (offset, output)
            });
        }
        while let Some(joined) = set.join_next().await {
            match joined {
                Ok((offset, output)) => out[start + offset] = output,
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
/// R36.10 page plan: chunks partition their sections in order, so one
/// ordered walk assigns every chunk its content's page span. Marker lines
/// are stripped from stored content (no `page`-token FTS noise) and
/// marker-only chunks are dropped, not stored blank; sections roll up their
/// kept chunks' spans. Texts without markers return untouched — non-PDF
/// chunking is byte-identical, including `§` numbering (which counts final
/// chunks).
fn apply_page_plan(
    metas: Vec<crate::chunking::SectionMeta>,
    chunks: Vec<crate::chunking::ChunkMeta>,
    cleaned: &str,
) -> (Vec<crate::chunking::SectionMeta>, Vec<crate::chunking::ChunkMeta>) {
    if !crate::extract::has_page_markers(cleaned) {
        return (metas, chunks);
    }
    let spans = chunk_page_spans(&chunks);
    let mut kept: Vec<crate::chunking::ChunkMeta> = Vec::with_capacity(chunks.len());
    let mut kept_spans: Vec<(u32, u32)> = Vec::with_capacity(chunks.len());
    for (mut c, span) in chunks.into_iter().zip(spans) {
        let stripped = crate::extract::strip_page_markers(&c.content);
        if stripped.trim().is_empty() {
            continue;
        }
        c.content = stripped;
        c.path = chunk_path_with_page(&c.path, span.0, span.1);
        kept.push(c);
        kept_spans.push(span);
    }
    let sec_spans = section_page_spans(metas.len(), &kept, &kept_spans);
    let metas = metas
        .into_iter()
        .enumerate()
        .map(|(i, mut m)| {
            let (s, e) = sec_spans.get(i).copied().unwrap_or((1, 1));
            m.path = section_path_with_page(&m.path, s, e);
            m
        })
        .collect();
    (metas, kept)
}

/// Per-chunk page spans in chunk order, carrying the running page across
/// chunks. Spans cover content lines only: a chunk opening with its marker
/// sits on that page, and a trailing marker with no content after it does
/// not extend the span (the running page still advances past it).
fn chunk_page_spans(chunks: &[crate::chunking::ChunkMeta]) -> Vec<(u32, u32)> {
    let mut page = 1u32;
    chunks
        .iter()
        .map(|c| {
            let (mut first, mut last) = (None, None);
            for line in c.content.lines() {
                if let Some(n) = crate::extract::page_marker_number(line) {
                    page = page.max(n);
                } else if !line.trim().is_empty() {
                    if first.is_none() {
                        first = Some(page);
                    }
                    last = Some(page);
                }
            }
            (first.unwrap_or(page), last.unwrap_or(page))
        })
        .collect()
}

/// Per-section page spans rolled up from member chunks; chunk-less sections
/// (only fully-empty docs) default to page 1.
fn section_page_spans(
    n_sections: usize,
    chunks: &[crate::chunking::ChunkMeta],
    spans: &[(u32, u32)],
) -> Vec<(u32, u32)> {
    let mut out: Vec<Option<(u32, u32)>> = vec![None; n_sections];
    for (c, (s, e)) in chunks.iter().zip(spans.iter()) {
        if let Some(idx) = c.section_idx.filter(|i| *i < n_sections) {
            out[idx] = Some(match out[idx] {
                None => (*s, *e),
                Some((ps, pe)) => (ps.min(*s), pe.max(*e)),
            });
        }
    }
    out.into_iter().map(|o| o.unwrap_or((1, 1))).collect()
}

fn page_suffix(start: u32, end: u32) -> String {
    if start >= end {
        format!("(p. {start})")
    } else {
        format!("(pp. {start}-{end})")
    }
}

/// Section paths always carry the page (root `""` becomes just the page —
/// the breadcrumb then names the page instead of reading empty).
fn section_path_with_page(path: &str, start: u32, end: u32) -> String {
    if path.is_empty() {
        page_suffix(start, end)
    } else {
        format!("{path} {}", page_suffix(start, end))
    }
}

/// Chunk paths keep the R28.1 contract: empty stays empty so reads still
/// fall back to the item's file path (the page rides the breadcrumb).
fn chunk_path_with_page(path: &str, start: u32, end: u32) -> String {
    if path.is_empty() {
        String::new()
    } else {
        format!("{path} {}", page_suffix(start, end))
    }
}

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
    // Markdown joins them (R38.1): the block splitter reads list/code
    // indentation, and cleaning drops lines, which would shift the stored
    // 1-based line ranges away from the raw item text. Extraction + distill
    // still run for markdown — only the cleaning step is skipped.
    let skip_clean = ctx.skips_extraction()
        || crate::markdown::is_markdown_filename(&ctx.external_id)
        || crate::markdown::is_markdown_filename(&ctx.base_ref);
    run_db(&ctx.data_dir, move |conn| {
        // F-03: delete+insert is one transaction — a crash between the
        // DELETEs and the re-inserts used to strand the item chunk-less.
        // Early `?` returns skip COMMIT and this fresh per-call connection
        // rolls back on drop; only the fall-through path commits.
        conn.execute("BEGIN IMMEDIATE", []).map_err(|e| e.to_string())?;
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
        // R36.10: PDF marker text gains breadcrumb page numbers + marker-free
        // chunk content; anything without markers passes through untouched.
        let (metas, chunk_metas) = apply_page_plan(metas, chunk_metas, &cleaned);
        // insert sections in order so parent idx maps correctly
        let mut section_ids: Vec<i64> = Vec::with_capacity(metas.len());
        for meta in &metas {
            let parent_db = meta.parent.and_then(|p| section_ids.get(p).copied());
            conn.execute(
                "INSERT INTO sections (item_id, parent_id, level, title, path, line_start, line_end, kind, meta, created_at, uid) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,datetime('now'),?10)",
                rusqlite::params![item_id, parent_db, meta.level, meta.title, meta.path, meta.line_start, meta.line_end, meta.kind, meta.meta, crate::db::new_uid()],
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
                "INSERT INTO chunks (item_id, kind, source_ref, content, section_id, level, path, line_start, line_end, created_at, uid) VALUES (?1,'document',?2,?3,?4,?5,?6,?7,?8,datetime('now'),?9)",
                rusqlite::params![item_id, r, cm.content, sec_db, cm.level, cm.path, cm.line_start, cm.line_end, crate::db::new_uid()],
            )
            .map_err(|e| e.to_string())?;
        }
        conn.execute("COMMIT", []).map_err(|e| e.to_string())?;
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

// B18 distillation (record bucket → Q&A units), hash-skipped.
// R22.1: uses the LLM distiller (rules fallback inside `Classifier::distill`).
async fn distill_item(ctx: &ClassifyCtx) -> Result<(), String> {
    let distill_enabled = crate::classifier::distill_enabled(&ctx.settings);
    // R33.4: structured files skip distillation too — paraphrase chunks are
    // the same noise class as extracted entities (and burn an LLM call).
    if ctx.skips_extraction()
        || !(crate::classifier::bucket_distills(&ctx.doc_type) && distill_enabled)
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
async fn fetch_embed_batch(data_dir: &str, source_id: i64, last_id: i64) -> Result<Vec<(i64, String, String)>, String> {
    run_db(data_dir, move |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT id, content, path FROM chunks \
                 WHERE id > ?1 AND embedding IS NULL AND ( \
                   item_id IN (SELECT id FROM ingested_items WHERE source_id = ?2) \
                   OR entity_id IN (SELECT e.id FROM entities e JOIN ingested_items i ON i.id = e.item_id WHERE i.source_id = ?2)) \
                 ORDER BY id LIMIT 64",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(rusqlite::params![last_id, source_id], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))
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
    let chunks: Vec<(i64, String, String)> = run_db(&ctx.data_dir, move |conn| {
        let mut stmt = conn.prepare("SELECT id, content, path FROM chunks WHERE (item_id=?1 OR entity_id IN (SELECT id FROM entities WHERE item_id=?1)) AND embedding IS NULL").unwrap();
        let rows = stmt.query_map([item_id], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))).unwrap().filter_map(|r| r.ok()).collect();
        Ok(rows)
    }).await?;
    if chunks.is_empty() {
        return Ok(());
    }
    let pending_ids: Vec<i64> = chunks.iter().map(|(id, _, _)| *id).collect();
    let contents: Vec<String> = chunks.iter().map(|(_, c, _)| c.clone()).collect();
    let embed_texts: Vec<String> = chunks.iter().map(|(_, c, p)| crate::markdown::embed_input(p, c)).collect();
    // call embedder (graceful fallback inside embed_gated on remote failure)
    let out = ctx.embedder.embed_gated(contents, embed_texts, gated, embed_min_signal).await.unwrap_or_else(|_| vec![None; pending_ids.len()]);
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

    #[tokio::test]
    async fn markdown_doc_chunks_persist_with_ranges() {
        // R38.1 wiring: a real `ClassifyCtx` through `store_doc_chunks` +
        // `create_summaries_inner` — sections/chunks/summaries carry the
        // markdown tree, line ranges, kinds and ADR meta (no sockets).
        let dir = tempdir().unwrap();
        let data_dir = dir.path().to_string_lossy().to_string();
        let db_path = crate::db::resolve_db_path(&data_dir);
        let conn = crate::db::init_db(&db_path).unwrap();
        conn.execute("INSERT INTO sources (name, connector, config, enabled, last_sync_cursor, error_count) VALUES ('s','folder','{}',1,'',0)", []).unwrap();
        let sid = conn.last_insert_rowid();
        let md = "---\ntitle: ADR 005\nstatus: accepted\n---\n\n# ADR 005\n\nintro line\n\n## Cancel\nbody here\n";
        conn.execute("INSERT INTO ingested_items (source_id, external_id, title, text, content_hash, author, stale) VALUES (?1,'adr-005.md','adr-005.md','t','h','',0)", [sid]).unwrap();
        let iid = conn.last_insert_rowid();
        drop(conn);
        let store = crate::secrets::SecretStore::new(dir.path().join("secrets.enc"));
        let settings = Arc::new(crate::settings::SettingsService::new(store));
        settings.set_data_dir(dir.path());
        let classifier = Arc::new(crate::classifier::Classifier::new(settings.clone()));
        let embedder = Arc::new(crate::embedder::Embedder::new(settings.clone()));
        let doc = IngestionDoc {
            external_id: "adr-005.md".to_string(),
            title: "adr-005.md".to_string(),
            text: md.to_string(),
            author: String::new(),
            updated_at: None,
            source_ref: "adr-005.md".to_string(),
        };
        let mut ctx = ClassifyCtx::new(&data_dir, sid, 0, &doc, false, settings, classifier, embedder);
        ctx.item_id = iid;
        ctx.doc_type = "decision_log".to_string();
        store_doc_chunks(&ctx).await.unwrap();
        let conn = crate::db::init_db(&db_path).unwrap();
        let secs: Vec<(String, String, i64, i64, String, String)> = conn
            .prepare("SELECT title, path, line_start, line_end, kind, meta FROM sections WHERE item_id=?1 ORDER BY id")
            .unwrap()
            .query_map([iid], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        assert_eq!(secs.len(), 3, "root + front matter + Cancel: {:?}", secs.iter().map(|s| &s.0).collect::<Vec<_>>());
        assert_eq!(secs[0].0, "ADR 005");
        assert_eq!(secs[0].1, "ADR 005");
        assert_eq!((secs[0].2, secs[0].3), (1, 12));
        assert!(secs[0].5.contains("\"adr_number\":\"5\""), "root meta: {}", secs[0].5);
        assert!(secs[0].5.contains("\"status\":\"accepted\""), "root meta: {}", secs[0].5);
        assert_eq!(secs[1].4, "front_matter");
        assert_eq!((secs[1].2, secs[1].3), (1, 4));
        assert_eq!(secs[2].1, "ADR 005 > Cancel");
        assert_eq!((secs[2].2, secs[2].3), (10, 12));
        let (ranged, total): (i64, i64) = conn
            .query_row("SELECT COUNT(CASE WHEN line_start > 0 THEN 1 END), COUNT(*) FROM chunks WHERE item_id=?1 AND kind='document'", [iid], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap();
        assert!(total >= 3, "root + FM + Cancel chunks");
        assert_eq!(ranged, total, "every markdown document chunk carries a line range");
        // Section-level entries exist for every section incl. the root (§4).
        create_summaries_inner(&conn, iid, md, "adr-005.md", false).unwrap();
        let summaries: i64 = conn.query_row("SELECT COUNT(*) FROM chunks WHERE item_id=?1 AND kind='section_summary'", [iid], |r| r.get(0)).unwrap();
        assert_eq!(summaries, 3, "root + FM + Cancel summaries");
        let root_sum: i64 = conn.query_row("SELECT COUNT(*) FROM chunks WHERE section_id=(SELECT id FROM sections WHERE item_id=?1 AND level=0) AND kind='section_summary'", [iid], |r| r.get(0)).unwrap();
        assert_eq!(root_sum, 1, "markdown root is summarized (prose empty roots skip)");
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
    fn resolve_config_restores_stripped_secrets() {
        // Sync-time regression: POST /sources strips secret fields into the
        // SecretStore, so the stored config has NO api_key/token key at all —
        // resolve_config must fill absent fields from the store, not just
        // "***set***" placeholders (those only exist in GET /config output,
        // never on disk). Without this every Linear/Jira/GDrive sync fails
        // with "requires api_key (or token)".
        let _lock = crate::secrets::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var("ANCHOR_SECRETS_NO_KEYRING").ok();
        std::env::set_var("ANCHOR_SECRETS_NO_KEYRING", "1");

        let dir = tempdir().unwrap();
        let data_dir = dir.path().to_string_lossy().to_string();
        let store = SecretStore::new(dir.path().join("secrets.enc"));
        store.set("source:7:api_key", "lin_api_real");

        // absent field (the as-stored shape) is filled from the store ...
        let out = resolve_config(&data_dir, 7, serde_json::json!({"team": "ENG"}));
        assert_eq!(out.get("api_key").and_then(|v| v.as_str()), Some("lin_api_real"));
        // ... so the connector now builds
        assert!(LinearConnector::new(&out).is_ok());
        // placeholder is still resolved ...
        let out = resolve_config(&data_dir, 7, serde_json::json!({"team": "ENG", "api_key": "***set***"}));
        assert_eq!(out.get("api_key").and_then(|v| v.as_str()), Some("lin_api_real"));
        // ... legacy plaintext is left untouched ...
        let out = resolve_config(&data_dir, 7, serde_json::json!({"team": "ENG", "api_key": "plain"}));
        assert_eq!(out.get("api_key").and_then(|v| v.as_str()), Some("plain"));
        // ... nothing is invented when the store is empty ...
        let out = resolve_config(&data_dir, 8, serde_json::json!({"team": "ENG"}));
        assert!(out.get("api_key").is_none());
        // ... and a token-only legacy source resolves through the alias
        store.set("source:9:token", "lin_api_legacy");
        let out = resolve_config(&data_dir, 9, serde_json::json!({"team": "ENG"}));
        assert_eq!(out.get("api_key").and_then(|v| v.as_str()), Some("lin_api_legacy"));
        assert!(LinearConnector::new(&out).is_ok());
        // ... and the same stripped shape resolves for every other API
        // connector (all had the identical sync failure through this one
        // choke point: Jira "requires base_url, email, token", GDrive
        // "requires token", REST bearer "requires token")
        store.set("source:11:token", "jira_tok");
        let out = resolve_config(&data_dir, 11, serde_json::json!({"base_url": "https://x.atlassian.net", "email": "a@b.c", "project": "PM"}));
        assert!(JiraConnector::new(&out).is_ok());
        store.set("source:12:token", "drive_tok");
        let out = resolve_config(&data_dir, 12, serde_json::json!({"folder_id": "fld123"}));
        assert!(GDriveConnector::new(&out).is_ok());
        store.set("source:13:token", "rest_tok");
        let out = resolve_config(&data_dir, 13, serde_json::json!({"base_url": "https://api.example.com", "list_path": "/items", "auth_mode": "bearer"}));
        assert!(RestConnector::new(&out).is_ok());
        // REST basic reads `password` — stripped at create, silently empty
        // at sync (remote 401 rather than a local error), now restored
        store.set("source:14:password", "rest_pw");
        let out = resolve_config(&data_dir, 14, serde_json::json!({"base_url": "https://api.example.com", "list_path": "/items", "auth_mode": "basic", "email": "a@b.c"}));
        assert_eq!(out.get("password").and_then(|v| v.as_str()), Some("rest_pw"));
        // GDrive `access_token` alias resolves the same way and the
        // connector accepts it (used to sit plaintext in the DB config)
        store.set("source:15:access_token", "drive_at");
        let out = resolve_config(&data_dir, 15, serde_json::json!({"folder_id": "fld123"}));
        assert_eq!(out.get("access_token").and_then(|v| v.as_str()), Some("drive_at"));
        assert!(GDriveConnector::new(&out).is_ok());

        match prev {
            Some(v) => std::env::set_var("ANCHOR_SECRETS_NO_KEYRING", v),
            None => std::env::remove_var("ANCHOR_SECRETS_NO_KEYRING"),
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

    /// R36.10 page-plan fixture: marker text through the real chunker —
    /// heading sections per page plus a leading root block, asserting the
    /// stored section/chunk paths and marker-free chunk bytes.
    fn page_plan_text() -> String {
        "[page 1]\nCover words before any heading.\n\n# Terms\nFirst page terms words.\n\n[page 2]\n# Scope\nSecond page scope words.\n".to_string()
    }

    #[tokio::test]
    async fn store_doc_chunks_pdf_pages_in_breadcrumbs() {
        let dir = tempdir().unwrap();
        let data_dir = dir.path().to_string_lossy().to_string();
        let db_path = crate::db::resolve_db_path(&data_dir);
        let conn = crate::db::init_db(&db_path).unwrap();
        conn.execute("INSERT INTO sources (name, connector, config, enabled, last_sync_cursor, error_count) VALUES ('s','folder','{}',1,'',0)", []).unwrap();
        let sid = conn.last_insert_rowid();
        conn.execute("INSERT INTO ingested_items (source_id, external_id, title, text, content_hash, author, stale) VALUES (?1,'terms.pdf','terms.pdf','t','h','',0)", [sid]).unwrap();
        let iid = conn.last_insert_rowid();
        drop(conn);
        let store = crate::secrets::SecretStore::new(dir.path().join("secrets.enc"));
        let settings = Arc::new(crate::settings::SettingsService::new(store));
        settings.set_data_dir(dir.path());
        let classifier = Arc::new(crate::classifier::Classifier::new(settings.clone()));
        let embedder = Arc::new(crate::embedder::Embedder::new(settings.clone()));
        let doc = IngestionDoc {
            external_id: "terms.pdf".to_string(),
            title: "terms.pdf".to_string(),
            text: page_plan_text(),
            author: String::new(),
            updated_at: None,
            source_ref: "terms.pdf".to_string(),
        };
        let mut ctx = ClassifyCtx::new(&data_dir, sid, 0, &doc, false, settings, classifier, embedder);
        ctx.item_id = iid;
        ctx.doc_type = "reference".to_string();
        store_doc_chunks(&ctx).await.unwrap();
        let conn = crate::db::init_db(&db_path).unwrap();
        let mut stmt = conn.prepare("SELECT path FROM sections WHERE item_id=?1 ORDER BY id").unwrap();
        let paths: Vec<String> = stmt.query_map([iid], |r| r.get(0)).unwrap().filter_map(|r| r.ok()).collect();
        assert!(paths.iter().any(|p| p == "(p. 1)"), "root section names its page: {paths:?}");
        // (R40.5: the prose splitter no longer prefixes top-level paths with
        // `" > "` — the page suffix appends to the bare heading instead.)
        assert!(paths.iter().any(|p| p == "Terms (p. 1)"), "page-1 section: {paths:?}");
        assert!(paths.iter().any(|p| p == "Scope (p. 2)"), "page-2 section: {paths:?}");
        let mut stmt = conn.prepare("SELECT content, path FROM chunks WHERE item_id=?1 AND kind='document' ORDER BY id").unwrap();
        let chunks: Vec<(String, String)> = stmt.query_map([iid], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().filter_map(|r| r.ok()).collect();
        assert!(!chunks.is_empty());
        for (content, path) in &chunks {
            assert!(!crate::extract::has_page_markers(content), "stored chunk content is marker-free: {content:?}");
            if path.is_empty() {
                continue;
            }
            assert!(path.ends_with("(p. 1)") || path.ends_with("(p. 2)"), "chunk path carries its page: {path:?}");
        }
        assert!(chunks.iter().any(|(_, p)| p.is_empty()), "root chunks keep the empty-path file fallback: {chunks:?}");
        assert!(chunks.iter().any(|(_, p)| p == "Terms (p. 1)"), "sectioned chunks carry pages: {chunks:?}");
    }

    #[tokio::test]
    async fn classify_windows_stores_rule_vocab_without_llm() {
        // R40.7 Tier-1 end of the hook: glossary-shaped doc text yields
        // active rule rows through classify_windows with no model running
        // (rules fallback per window); structured files store nothing.
        // Dead-port base + short timeout: the per-window LLM attempt must
        // fail fast (sandboxed connects hang to the timeout).
        let _guard = crate::retrieval::ENV_LOCK.lock().unwrap();
        std::env::set_var("ANCHOR_CLASSIFIER_BASE_URL", "http://localhost:1");
        std::env::set_var("ANCHOR_CLASSIFIER_TIMEOUT", "2");
        let dir = tempdir().unwrap();
        let data_dir = dir.path().to_string_lossy().to_string();
        let db_path = crate::db::resolve_db_path(&data_dir);
        let conn = crate::db::init_db(&db_path).unwrap();
        conn.execute("INSERT INTO sources (name, connector, config, enabled, last_sync_cursor, error_count) VALUES ('s','folder','{}',1,'',0)", []).unwrap();
        let sid = conn.last_insert_rowid();
        conn.execute("INSERT INTO jobs (source_id, kind, status, total, processed, result, created_at) VALUES (?1,'sync','running',1,0,'{}',datetime('now'))", [sid]).unwrap();
        let jid = conn.last_insert_rowid();
        conn.execute("INSERT INTO ingested_items (source_id, external_id, title, text, content_hash, author, stale) VALUES (?1,'terms.md','terms.md','t','h','',0)", [sid]).unwrap();
        let iid = conn.last_insert_rowid();
        conn.execute("INSERT INTO ingested_items (source_id, external_id, title, text, content_hash, author, stale) VALUES (?1,'values.yaml','values.yaml','t','h','',0)", [sid]).unwrap();
        let yid = conn.last_insert_rowid();
        drop(conn);
        let store = crate::secrets::SecretStore::new(dir.path().join("secrets.enc"));
        let settings = Arc::new(crate::settings::SettingsService::new(store));
        settings.set_data_dir(dir.path());
        let classifier = Arc::new(crate::classifier::Classifier::new(settings.clone()));
        let embedder = Arc::new(crate::embedder::Embedder::new(settings.clone()));
        let text = "Each Long Form (CODE) row carries a reason.\n\n## Glossary\nCA - Direct Debit record\n".to_string();
        let doc = IngestionDoc {
            external_id: "terms.md".to_string(),
            title: "terms.md".to_string(),
            text: text.clone(),
            author: String::new(),
            updated_at: None,
            source_ref: "terms.md".to_string(),
        };
        let mut ctx = ClassifyCtx::new(&data_dir, sid, jid, &doc, false, settings.clone(), classifier.clone(), embedder.clone());
        ctx.item_id = iid;
        ctx.doc_type = "reference".to_string();
        ctx.classify_windows().await.unwrap();
        let ydoc = IngestionDoc {
            external_id: "values.yaml".to_string(),
            title: "values.yaml".to_string(),
            text,
            author: String::new(),
            updated_at: None,
            source_ref: "values.yaml".to_string(),
        };
        let mut yctx = ClassifyCtx::new(&data_dir, sid, jid, &ydoc, false, settings, classifier, embedder);
        yctx.item_id = yid;
        yctx.doc_type = "reference".to_string();
        yctx.classify_windows().await.unwrap();
        let conn = crate::db::init_db(&db_path).unwrap();
        let rows: Vec<(String, String, String, String)> = conn
            .prepare("SELECT term, expansion, origin, status FROM vocabulary WHERE item_id=?1 ORDER BY term")
            .unwrap()
            .query_map([iid], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        assert!(
            rows.contains(&("ca".to_string(), "direct debit record".to_string(), "rule".to_string(), "active".to_string())),
            "glossary row: {rows:?}"
        );
        assert!(
            rows.contains(&("code".to_string(), "long form".to_string(), "rule".to_string(), "active".to_string())),
            "parenthetical row: {rows:?}"
        );
        let yn: i64 = conn.query_row("SELECT COUNT(*) FROM vocabulary WHERE item_id=?1", [yid], |r| r.get(0)).unwrap();
        assert_eq!(yn, 0, "structured files carry no vocabulary");
        std::env::remove_var("ANCHOR_CLASSIFIER_BASE_URL");
        std::env::remove_var("ANCHOR_CLASSIFIER_TIMEOUT");
    }

    #[test]
    fn page_plan_unit_spans_suffixes_and_skips() {
        use crate::chunking::{ChunkMeta, SectionMeta};
        let metas = vec![
            SectionMeta { title: String::new(), level: 0, parent: None, path: String::new(), line_start: 0, line_end: 0, kind: "section".to_string(), meta: "{}".to_string() },
            SectionMeta { title: "Late".to_string(), level: 1, parent: None, path: "Late".to_string(), line_start: 0, line_end: 0, kind: "section".to_string(), meta: "{}".to_string() },
        ];
        let chunks = vec![
            ChunkMeta { content: "[page 2]\nspanning words".to_string(), section_idx: Some(1), level: 1, path: "Late".to_string(), line_start: 0, line_end: 0 },
            ChunkMeta { content: "more words\n[page 3]\ntail".to_string(), section_idx: Some(1), level: 1, path: "Late".to_string(), line_start: 0, line_end: 0 },
            ChunkMeta { content: "[page 4]".to_string(), section_idx: Some(0), level: 0, path: String::new(), line_start: 0, line_end: 0 },
        ];
        let (metas, chunks) = apply_page_plan(metas, chunks, "[page 2]\nx");
        assert_eq!(metas[0].path, "(p. 1)", "chunk-less root defaults to page 1");
        assert_eq!(metas[1].path, "Late (pp. 2-3)", "section rolls up member spans");
        assert_eq!(chunks.len(), 2, "marker-only chunk is skipped, not stored blank");
        assert_eq!(chunks[0].path, "Late (p. 2)");
        assert_eq!(chunks[1].path, "Late (pp. 2-3)");
        assert!(!chunks.iter().any(|c| crate::extract::has_page_markers(&c.content)));
        // No markers anywhere: inputs pass through untouched (same shapes).
        let metas = vec![SectionMeta { title: String::new(), level: 0, parent: None, path: String::new(), line_start: 0, line_end: 0, kind: "section".to_string(), meta: "{}".to_string() }];
        let chunks = vec![ChunkMeta { content: "plain words".to_string(), section_idx: Some(0), level: 0, path: String::new(), line_start: 0, line_end: 0 }];
        let (metas, chunks) = apply_page_plan(metas, chunks, "plain words");
        assert_eq!(metas[0].path, "");
        assert_eq!(chunks[0].content, "plain words");
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

        // Legacy row: a pre-migration item carries content but no fingerprint
        // (migration 18 backfills processed_hash=content_hash for such rows).
        let text_a = std::fs::read_to_string(folder.join("a.md")).unwrap();
        let db_path = crate::db::resolve_db_path(&data_dir);
        let conn = crate::db::init_db(&db_path).unwrap();
        conn.execute(
            "INSERT INTO ingested_items (source_id, external_id, title, text, content_hash, processed_hash) VALUES (?1,'a.md','a.md',?2,?3,?3)",
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

    #[tokio::test]
    async fn interrupted_item_reprocesses_on_next_sync() {
        // F-03: an item whose sync died between upsert and finalize (new
        // text+hash stored, completion markers stale) must re-process on the
        // next sync and converge to the uninterrupted state — same entities,
        // same chunks, markers restored.
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
        let db_path = crate::db::resolve_db_path(&data_dir);

        let (items, _, _, _, _) = run_folder_once(&pipe, &data_dir, sid, &cfg, false).await;
        assert_eq!(items, 2);
        let conn = crate::db::init_db(&db_path).unwrap();
        let marked: i64 = conn.query_row("SELECT COUNT(*) FROM ingested_items WHERE source_id=?1 AND processed_hash IS NOT NULL AND processed_hash = content_hash", [sid], |r| r.get(0)).unwrap();
        assert_eq!(marked, 2, "a clean sync must set the completion marker");
        let aid: i64 = conn.query_row("SELECT id FROM ingested_items WHERE source_id=?1 AND external_id='a.md'", [sid], |r| r.get(0)).unwrap();
        let ent_a: i64 = conn.query_row("SELECT COUNT(*) FROM entities WHERE item_id=?1", [aid], |r| r.get(0)).unwrap();
        let chunks_a: i64 = conn.query_row("SELECT COUNT(*) FROM chunks WHERE item_id=?1 OR entity_id IN (SELECT id FROM entities WHERE item_id=?1)", [aid], |r| r.get(0)).unwrap();
        assert!(ent_a > 0 && chunks_a > 0, "seed sync must produce entities+chunks for a.md");
        // Simulate a cancel between upsert and finalize: stored content is
        // current but the completion markers + window hashes are stale, the
        // fingerprint is gone, and no entities were kept.
        conn.execute("UPDATE ingested_items SET processed_hash='interrupted', file_mtime_ms=NULL, file_size=NULL, window_hashes='{}', distill_hashes='{}' WHERE id=?1", [aid]).unwrap();
        conn.execute("DELETE FROM entities WHERE item_id=?1", [aid]).unwrap();
        drop(conn);

        let (items, _, _, unchanged, _) = run_folder_once(&pipe, &data_dir, sid, &cfg, false).await;
        assert_eq!((items, unchanged), (1, 1), "only the interrupted file reprocesses; b.md still skips");
        let conn = crate::db::init_db(&db_path).unwrap();
        let ent_a2: i64 = conn.query_row("SELECT COUNT(*) FROM entities WHERE item_id=?1", [aid], |r| r.get(0)).unwrap();
        let chunks_a2: i64 = conn.query_row("SELECT COUNT(*) FROM chunks WHERE item_id=?1 OR entity_id IN (SELECT id FROM entities WHERE item_id=?1)", [aid], |r| r.get(0)).unwrap();
        assert_eq!(ent_a2, ent_a, "entities must converge to the uninterrupted count");
        assert_eq!(chunks_a2, chunks_a, "chunks must converge to the uninterrupted count");
        let (processed, stored): (Option<String>, String) = conn.query_row("SELECT processed_hash, content_hash FROM ingested_items WHERE id=?1", [aid], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!(processed.as_deref(), Some(stored.as_str()), "marker must be restored");
        let fp: i64 = conn.query_row("SELECT COUNT(*) FROM ingested_items WHERE id=?1 AND file_mtime_ms IS NOT NULL AND file_size IS NOT NULL", [aid], |r| r.get(0)).unwrap();
        assert_eq!(fp, 1, "fingerprint must be restored");
    }

    #[tokio::test]
    async fn finalize_checks_cancel_before_every_phase() {
        // F-08: a cancelled job must not start finalize work — the first gate
        // fires before any phase writes (the same gate stands before all six).
        let dir = tempdir().unwrap();
        let data_dir = dir.path().to_string_lossy().to_string();
        let db_path = crate::db::resolve_db_path(&data_dir);
        let conn = crate::db::init_db(&db_path).unwrap();
        conn.execute("INSERT INTO sources (name, connector, config, enabled, last_sync_cursor, error_count) VALUES ('s','folder','{}',1,'',0)", []).unwrap();
        let sid = conn.last_insert_rowid();
        conn.execute("INSERT INTO ingested_items (source_id, external_id, title, text, content_hash, author, stale) VALUES (?1,'a.md','a.md','body text here','h','',0)", [sid]).unwrap();
        let iid = conn.last_insert_rowid();
        conn.execute("INSERT INTO jobs (source_id, kind, status, total, processed, result, created_at) VALUES (?1,'sync','cancelled',0,0,'{}',datetime('now'))", [sid]).unwrap();
        let jid = conn.last_insert_rowid();
        drop(conn);
        let store = crate::secrets::SecretStore::new(dir.path().join("secrets.enc"));
        let settings = Arc::new(crate::settings::SettingsService::new(store));
        settings.set_data_dir(dir.path());
        let classifier = Arc::new(crate::classifier::Classifier::new(settings.clone()));
        let embedder = Arc::new(crate::embedder::Embedder::new(settings.clone()));
        let doc = IngestionDoc { external_id: "a.md".to_string(), title: "a.md".to_string(), text: "body text here".to_string(), author: String::new(), updated_at: None, source_ref: "a.md".to_string() };
        let mut ctx = ClassifyCtx::new(&data_dir, sid, jid, &doc, false, settings, classifier, embedder);
        ctx.item_id = iid;
        ctx.doc_type = "reference".to_string();
        let mut perf = PhaseTimes::new();
        let err = ctx.finalize(&mut perf).await.unwrap_err();
        assert_eq!(err, "job cancelled");
        let conn = crate::db::init_db(&db_path).unwrap();
        let chunks: i64 = conn.query_row("SELECT COUNT(*) FROM chunks WHERE item_id=?1", [iid], |r| r.get(0)).unwrap();
        assert_eq!(chunks, 0, "no finalize phase may write after cancel");
    }

    #[tokio::test]
    async fn folder_sync_aborts_immediately_when_cancelled_upfront() {
        // F-08: the between-documents gate — a job cancelled before the sync
        // starts processes nothing.
        let data_tmp = tempdir().unwrap();
        let folder_tmp = tempdir().unwrap();
        let data_dir = data_tmp.path().to_string_lossy().to_string();
        let folder = folder_tmp.path();
        write_doc(folder, "a.md", "# Alpha\n\nWe decided to use sqlite for vectors.\n");
        let (settings, sid) = seed_folder_source(&data_dir, folder);
        let classifier = Arc::new(crate::classifier::Classifier::new(settings.clone()));
        let embedder = Arc::new(crate::embedder::Embedder::new(settings.clone()));
        let pipe = Pipeline::new_for_test(classifier, embedder, settings, data_dir.clone());
        let cfg = serde_json::json!({"path": folder.to_string_lossy()});
        let db_path = crate::db::resolve_db_path(&data_dir);
        let conn = crate::db::init_db(&db_path).unwrap();
        conn.execute("INSERT INTO jobs (source_id, kind, status, total, processed, result, created_at) VALUES (?1,'sync','cancelled',0,0,'{}',datetime('now'))", [sid]).unwrap();
        let jid = conn.last_insert_rowid();
        drop(conn);
        let err = pipe.run_folder_sync(sid, jid, false, &cfg, &data_dir).await.unwrap_err();
        assert_eq!(err, "job cancelled");
        assert_eq!(table_count(&data_dir, "ingested_items"), 0);
        assert_eq!(table_count(&data_dir, "entities"), 0);
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

    #[tokio::test]
    async fn buckets_record_may_emit_while_reference_yields_nothing_degraded() {
        // R33.6 §8(b): end to end through folder sync with the classifier on
        // closed ports (deterministic rules path, no Ollama): a record file
        // yields decision/action entities while a reference file yields none
        // (its chunks still index). Stored doc_type is the bucket name.
        let data_tmp = tempdir().unwrap();
        let folder_tmp = tempdir().unwrap();
        let data_dir = data_tmp.path().to_string_lossy().to_string();
        let folder = folder_tmp.path();
        write_doc(folder, "r.md", "# Sprint retro\n\nAttendees: Ana, Bo.\n\nWe decided to use sqlite for vectors. Next step: review on Friday.\n");
        write_doc(folder, "s.md", "# Widget spec\n\nThe widget renders blue panels on load. Operators file tickets through the portal.\n");
        let (settings, sid) = seed_folder_source(&data_dir, folder);
        let classifier = Arc::new(crate::classifier::Classifier::new(settings.clone()));
        let embedder = Arc::new(crate::embedder::Embedder::new(settings.clone()));
        let pipe = Pipeline::new_for_test(classifier, embedder, settings, data_dir.clone());
        let cfg = serde_json::json!({"path": folder.to_string_lossy()});
        let (items, entities, _, _, _) = run_folder_once(&pipe, &data_dir, sid, &cfg, false).await;
        assert_eq!((items, entities), (2, 2));
        let db_path = crate::db::resolve_db_path(&data_dir);
        let conn = crate::db::init_db(&db_path).unwrap();
        let dt: String = conn.query_row("SELECT doc_type FROM ingested_items WHERE source_id=?1 AND external_id='r.md'", [sid], |r| r.get(0)).unwrap();
        assert_eq!(dt, "record");
        let kinds: Vec<String> = conn.prepare("SELECT kind FROM entities WHERE item_id=(SELECT id FROM ingested_items WHERE source_id=?1 AND external_id='r.md')").unwrap().query_map([sid], |r| r.get(0)).unwrap().filter_map(|r| r.ok()).collect();
        assert!(kinds.contains(&"decision".to_string()), "record file must yield a decision: {kinds:?}");
        assert!(kinds.contains(&"action".to_string()), "record file must yield an action: {kinds:?}");
        let dt: String = conn.query_row("SELECT doc_type FROM ingested_items WHERE source_id=?1 AND external_id='s.md'", [sid], |r| r.get(0)).unwrap();
        assert_eq!(dt, "reference");
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM entities WHERE item_id=(SELECT id FROM ingested_items WHERE source_id=?1 AND external_id='s.md')", [sid], |r| r.get(0)).unwrap();
        assert_eq!(n, 0, "reference file yields no entities without the LLM");
        let chunks: i64 = conn.query_row("SELECT COUNT(*) FROM chunks WHERE item_id=(SELECT id FROM ingested_items WHERE source_id=?1 AND external_id='s.md')", [sid], |r| r.get(0)).unwrap();
        assert!(chunks >= 1, "reference chunks still index");
        let doc_kinds: i64 = conn.query_row("SELECT COUNT(*) FROM entities WHERE kind='document'", [], |r| r.get(0)).unwrap();
        assert_eq!(doc_kinds, 0, "new extraction never emits document");
    }

    #[tokio::test]
    async fn legacy_prose_entities_conflicts_review_survive_bucket_switch() {
        // R33.6 §8(c): unlike R33.2 code (purged), legacy prose passes
        // through read-only — entities, conflicts, disputes, and the legacy
        // doc_type all survive a re-sync; only a force reclassify re-buckets.
        let data_tmp = tempdir().unwrap();
        let folder_tmp = tempdir().unwrap();
        let data_dir = data_tmp.path().to_string_lossy().to_string();
        let folder = folder_tmp.path();
        write_doc(folder, "p.md", "# Sprint retro\n\nAttendees: Ana, Bo.\n\nWe decided to use sqlite for vectors. Next step: review on Friday.\n");
        let (settings, sid) = seed_folder_source(&data_dir, folder);
        let classifier = Arc::new(crate::classifier::Classifier::new(settings.clone()));
        let embedder = Arc::new(crate::embedder::Embedder::new(settings.clone()));
        let pipe = Pipeline::new_for_test(classifier, embedder, settings, data_dir.clone());
        let cfg = serde_json::json!({"path": folder.to_string_lossy()});
        let db_path = crate::db::resolve_db_path(&data_dir);
        run_folder_once(&pipe, &data_dir, sid, &cfg, false).await;
        // Downgrade the item to a legacy label + attach review/conflict state.
        let conn = crate::db::init_db(&db_path).unwrap();
        let iid: i64 = conn.query_row("SELECT id FROM ingested_items WHERE source_id=?1 AND external_id='p.md'", [sid], |r| r.get(0)).unwrap();
        conn.execute("UPDATE ingested_items SET doc_type='documentation' WHERE id=?1", [iid]).unwrap();
        let eids: Vec<i64> = conn.prepare("SELECT id FROM entities WHERE item_id=?1 ORDER BY id").unwrap().query_map([iid], |r| r.get(0)).unwrap().filter_map(|r| r.ok()).collect();
        assert_eq!(eids.len(), 2);
        conn.execute("UPDATE entities SET dispute_count=2, status='disputed' WHERE id=?1", [eids[0]]).unwrap();
        conn.execute("INSERT INTO disputes (entity_id, reason, user) VALUES (?1,'stale number','tester')", [eids[0]]).unwrap();
        conn.execute("INSERT INTO relationships (from_entity_id, to_entity_id, kind, source_ref, confidence) VALUES (?1,?2,'conflicts_with','p.md',0.9)", rusqlite::params![eids[0], eids[1]]).unwrap();
        drop(conn);
        // Unchanged re-sync: everything passes through byte-identical.
        let (items, _, _, unchanged, _) = run_folder_once(&pipe, &data_dir, sid, &cfg, false).await;
        assert_eq!((items, unchanged), (0, 1));
        let conn = crate::db::init_db(&db_path).unwrap();
        let dt: String = conn.query_row("SELECT doc_type FROM ingested_items WHERE id=?1", [iid], |r| r.get(0)).unwrap();
        assert_eq!(dt, "documentation", "non-force re-sync never rewrites the legacy label");
        let ents: Vec<(String, String, i64, String)> = conn.prepare("SELECT kind, summary, dispute_count, status FROM entities WHERE item_id=?1 ORDER BY id").unwrap().query_map([iid], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).unwrap().filter_map(|r| r.ok()).collect();
        assert_eq!(ents.len(), 2, "no purge of prose entities");
        assert_eq!(ents[0].2, 2);
        assert_eq!(ents[0].3, "disputed");
        let rels: i64 = conn.query_row("SELECT COUNT(*) FROM relationships WHERE kind='conflicts_with'", [], |r| r.get(0)).unwrap();
        assert_eq!(rels, 1, "conflict links survive");
        let disputes: i64 = conn.query_row("SELECT COUNT(*) FROM disputes", [], |r| r.get(0)).unwrap();
        assert_eq!(disputes, 1, "dispute rows survive");
        drop(conn);
        // Force reclassify re-buckets deterministically and re-extracts with
        // decision/action/note kinds only.
        run_folder_once(&pipe, &data_dir, sid, &cfg, true).await;
        let conn = crate::db::init_db(&db_path).unwrap();
        let dt: String = conn.query_row("SELECT doc_type FROM ingested_items WHERE id=?1", [iid], |r| r.get(0)).unwrap();
        assert_eq!(dt, "record", "force re-buckets the legacy label");
        let kinds: Vec<String> = conn.prepare("SELECT DISTINCT kind FROM entities WHERE item_id=?1").unwrap().query_map([iid], |r| r.get(0)).unwrap().filter_map(|r| r.ok()).collect();
        assert!(!kinds.is_empty());
        assert!(kinds.iter().all(|k| k == "decision" || k == "action" || k == "note"), "re-extracted kinds: {kinds:?}");
    }

    /// R45.1 counting mock: an OpenAI-compatible server that counts
    /// `chat/completions` requests (answers an empty extraction) and 404s
    /// everything else, so embeddings degrade to keyword-only.
    async fn spawn_counting_chat_mock() -> (u16, Arc<std::sync::atomic::AtomicUsize>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = calls.clone();
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = listener.accept().await {
                let counter = counter.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 65536];
                    let n = sock.read(&mut buf).await.unwrap_or(0);
                    let head = String::from_utf8_lossy(&buf[..n]);
                    let first = head.lines().next().unwrap_or("");
                    let (status, body) = if first.contains("chat/completions") {
                        counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        ("200 OK", r#"{"choices":[{"message":{"content":"{\"items\":[],\"units\":[]}"}}]}"#)
                    } else {
                        ("404 Not Found", "{}")
                    };
                    let resp = format!("HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len());
                    let _ = sock.write_all(resp.as_bytes()).await;
                });
            }
        });
        (port, calls)
    }

    fn copy_tree(from: &std::path::Path, to: &std::path::Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap().flatten() {
            let target = to.join(entry.file_name());
            if entry.path().is_dir() {
                copy_tree(&entry.path(), &target);
            } else {
                std::fs::copy(entry.path(), target).unwrap();
            }
        }
    }

    async fn sync_with_new_job(pipe: &Pipeline, db_path: &std::path::Path, sid: i64, force: bool) {
        let conn = crate::db::init_db(db_path).unwrap();
        conn.execute("INSERT INTO jobs (source_id, kind, status, total, processed, result, created_at) VALUES (?1,'sync','running',0,0,'{}',datetime('now'))", [sid]).unwrap();
        let jid = conn.last_insert_rowid();
        drop(conn);
        pipe.sync_source(sid, jid, force).await;
        let conn = crate::db::init_db(db_path).unwrap();
        let (status, err): (String, Option<String>) = conn.query_row("SELECT status, error FROM jobs WHERE id=?1", [jid], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!(status, "done", "sync failed: {err:?}");
    }

    #[tokio::test]
    async fn default_ingest_of_sample_makes_zero_chat_calls() {
        // R45.1 DoD: a `sample/` folder sync with default settings never
        // calls the chat model (rules extraction, no distillation), while
        // opting into `extract_entities=llm` + `distill_enabled` does — the
        // second half proves the counter actually sees model traffic.
        let (port, calls) = spawn_counting_chat_mock().await;
        let dir = tempdir().unwrap();
        let corpus = dir.path().join("corpus");
        copy_tree(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../sample"), &corpus);
        let data_dir = dir.path().join("data").to_string_lossy().to_string();
        std::fs::create_dir_all(&data_dir).unwrap();
        let db_path = crate::db::resolve_db_path(&data_dir);
        let conn = crate::db::init_db(&db_path).unwrap();
        let cfg = serde_json::json!({"path": corpus.to_string_lossy()}).to_string();
        conn.execute("INSERT INTO sources (name, connector, config, enabled, last_sync_cursor, error_count) VALUES ('sample','folder',?1,1,'',0)", [cfg]).unwrap();
        let sid = conn.last_insert_rowid();
        let store = crate::secrets::SecretStore::new(dir.path().join("secrets.enc"));
        let settings = Arc::new(crate::settings::SettingsService::new(store));
        settings.set_data_dir(std::path::Path::new(&data_dir));
        let mock = format!("http://127.0.0.1:{port}");
        for key in ["classifier_base_url", "embed_base_url", "ollama_base_url"] {
            settings.set(key, &mock, &conn).unwrap();
        }
        drop(conn);
        let classifier = Arc::new(crate::classifier::Classifier::new(settings.clone()));
        let embedder = Arc::new(crate::embedder::Embedder::new(settings.clone()));
        let pipe = Pipeline::new_for_test(classifier, embedder, settings.clone(), data_dir.clone());

        sync_with_new_job(&pipe, &db_path, sid, false).await;
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0, "default ingest made chat calls");
        let conn = crate::db::init_db(&db_path).unwrap();
        let items: i64 = conn.query_row("SELECT COUNT(*) FROM ingested_items WHERE source_id=?1", [sid], |r| r.get(0)).unwrap();
        let entities: i64 = conn.query_row("SELECT COUNT(*) FROM entities", [], |r| r.get(0)).unwrap();
        let distilled: i64 = conn.query_row("SELECT COUNT(*) FROM chunks WHERE kind='distilled'", [], |r| r.get(0)).unwrap();
        assert!(items >= 5, "sample corpus ingested: {items}");
        assert!(entities > 0, "rules extraction still stores entities");
        assert_eq!(distilled, 0, "distillation is opt-in");
        assert!(!crate::classifier::chat_model_needed(&settings));

        settings.set("extract_entities", "llm", &conn).unwrap();
        settings.set("distill_enabled", "true", &conn).unwrap();
        drop(conn);
        assert!(crate::classifier::chat_model_needed(&settings));
        sync_with_new_job(&pipe, &db_path, sid, true).await;
        assert!(calls.load(std::sync::atomic::Ordering::SeqCst) > 0, "opt-in llm mode reaches the model");
    }

    #[tokio::test]
    async fn extract_entities_off_stores_no_entities() {
        let dir = tempdir().unwrap();
        let data_dir = dir.path().to_string_lossy().to_string();
        let db_path = crate::db::resolve_db_path(&data_dir);
        let conn = crate::db::init_db(&db_path).unwrap();
        conn.execute("INSERT INTO sources (name, connector, config, enabled, last_sync_cursor, error_count) VALUES ('s','folder','{}',1,'',0)", []).unwrap();
        let sid = conn.last_insert_rowid();
        let store = crate::secrets::SecretStore::new(dir.path().join("secrets.enc"));
        let settings = Arc::new(crate::settings::SettingsService::new(store));
        settings.set_data_dir(dir.path());
        settings.set("extract_entities", "off", &conn).unwrap();
        let text = "# Meeting notes\nAttendees: Maya, Alex\n\nWe decided to migrate billing to Stripe.\n\n- [ ] Alex: draft the spec\n";
        let hash = crate::hashing::content_hash(text);
        conn.execute("INSERT INTO ingested_items (source_id, external_id, title, text, content_hash, author, stale) VALUES (?1,'meeting.md','meeting.md',?2,?3,'',0)", rusqlite::params![sid, text, hash]).unwrap();
        conn.execute("INSERT INTO jobs (source_id, kind, status, total, processed, result, created_at) VALUES (?1,'sync','running',1,0,'{}',datetime('now'))", [sid]).unwrap();
        let jid = conn.last_insert_rowid();
        drop(conn);
        let classifier = Arc::new(crate::classifier::Classifier::new(settings.clone()));
        let embedder = Arc::new(crate::embedder::Embedder::new(settings.clone()));
        let pipe = Pipeline::new_for_test(classifier, embedder, settings, data_dir.clone());
        let doc = IngestionDoc {
            external_id: "meeting.md".to_string(),
            title: "meeting.md".to_string(),
            text: text.to_string(),
            author: String::new(),
            updated_at: None,
            source_ref: "meeting.md".to_string(),
        };
        let n = pipe.classify_and_store(&data_dir, sid, jid, &doc, false).await.unwrap();
        assert_eq!(n, 0);
        let conn = crate::db::init_db(&db_path).unwrap();
        let entities: i64 = conn.query_row("SELECT COUNT(*) FROM entities", [], |r| r.get(0)).unwrap();
        assert_eq!(entities, 0, "off mode stores no entities");
        let chunks: i64 = conn.query_row("SELECT COUNT(*) FROM chunks WHERE kind='document'", [], |r| r.get(0)).unwrap();
        assert!(chunks >= 1, "document chunks are still stored");
    }
}
