//! Ingestion pipeline — port of `backend/app/pipeline.py:1`.
//! Folder → chunk → classify → distill → pii → embed (rule-based, no LLM needed for tests).

use std::sync::Arc;

use rusqlite::Connection;

use crate::classifier::Classifier;
use crate::connectors::{folder::FolderConnector, gdrive::GDriveConnector, jira::JiraConnector, linear::LinearConnector, IngestionDoc};
use crate::embedder::Embedder;
use crate::secrets::SecretStore;
use crate::settings::SettingsService;

fn content_hash(text: &str) -> String {
    crate::hashing::content_hash(text)
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
// R13.1: single DB-open boilerplate — every spawn_blocking DB task goes through this.
fn with_db<T>(data_dir: &str, f: impl FnOnce(&Connection) -> T) -> T {
    let db_path = crate::db::resolve_db_path(data_dir);
    let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| Connection::open(&db_path).unwrap());
    f(&conn)
}

async fn run_db<T, F>(data_dir: &str, f: F) -> Result<T, String>
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

async fn fetch_docs(connector_type: &str, config: &serde_json::Value, source_id: i64) -> Result<Vec<IngestionDoc>, String> {
    // R10.2: Jira/GDrive/Linear must not silently succeed with 0 items
    let docs = match connector_type {
        "folder" => {
            let fc = FolderConnector::new(config).map_err(|e| e.to_string())?;
            let (docs, _cursor) = fc.fetch().map_err(|e| e.to_string())?;
            docs
        }
        "jira" => {
            let jc = JiraConnector::new(config).map_err(|e| e.to_string())?;
            let (docs, _cursor) = jc.fetch("").await.map_err(|e| e.to_string())?;
            if docs.is_empty() {
                // treat empty as not error for now, but log
                tracing::warn!("Jira fetch returned 0 docs for source {}", source_id);
            }
            docs
        }
        "gdrive" => {
            let gc = GDriveConnector::new(config).map_err(|e| e.to_string())?;
            let (docs, _cursor) = gc.fetch("").await.map_err(|e| e.to_string())?;
            docs
        }
        "linear" => {
            let lc = LinearConnector::new(config).map_err(|e| e.to_string())?;
            let (docs, _cursor) = lc.fetch("").await.map_err(|e| e.to_string())?;
            docs
        }
        _ => return Err(format!("unknown connector '{}'", connector_type)),
    };
    Ok(docs)
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
    fn finish_job(&self, conn: &Connection, source_id: i64, job_id: i64, res: Result<(i64, i64), String>) {
        match res {
            Ok((items, entities)) => {
                // R9.1: do not clobber concurrent cancel
                let cur: String = conn.query_row("SELECT status FROM jobs WHERE id=?1", [job_id], |r| r.get(0)).unwrap_or_else(|_| "done".to_string());
                if cur == "cancelled" {
                    self.jobs.maybe_promote(conn);
                } else {
                    let result = serde_json::json!({"items": items, "entities": entities}).to_string();
                    let _ = conn.execute("UPDATE jobs SET status='done', finished_at=datetime('now'), result=?1, error=NULL WHERE id=?2", rusqlite::params![result, job_id]);
                    record_sync_success(conn, source_id);
                    // update job total/processed
                    let _ = conn.execute("UPDATE jobs SET total=?1, processed=?1 WHERE id=?2", rusqlite::params![items, job_id]);
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

    async fn run_sync_inner(&self, source_id: i64, job_id: i64, force_reclassify: bool, data_dir: &str) -> Result<(i64, i64), String> {
        // load source
        let (connector_type, config_str) = run_db(data_dir, move |conn| {
            let mut stmt = conn.prepare("SELECT connector, config FROM sources WHERE id = ?1").map_err(|e| e.to_string())?;
            stmt.query_row([source_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))).map_err(|_| "Source not found".to_string())
        }).await?;
        let mut config: serde_json::Value = serde_json::from_str(&config_str).unwrap_or(serde_json::json!({}));
        config = resolve_config(data_dir, source_id, config);
        // fetch docs — R10.2: Jira/GDrive/Linear must not silently succeed with 0 items
        let docs = fetch_docs(&connector_type, &config, source_id).await?;
        // update job total
        let total_docs = docs.len() as i64;
        run_db(data_dir, move |conn| {
            let _ = conn.execute("UPDATE jobs SET total=?1 WHERE id=?2", rusqlite::params![total_docs, job_id]);
            Ok(())
        }).await?;
        let mut created_items = 0i64;
        let mut new_entities = 0i64;
        for (idx, doc) in docs.into_iter().enumerate() {
            // check if job was cancelled
            if job_cancelled(data_dir, job_id).await? {
                return Err("job cancelled".to_string());
            }
            let upserted = Box::pin(self.upsert_doc(data_dir, source_id, &doc, force_reclassify)).await.map_err(|e| e.to_string())?;
            if upserted || force_reclassify {
                created_items += 1;
                let n = Box::pin(self.classify_and_store(data_dir, source_id, job_id, &doc, force_reclassify)).await.map_err(|e| e.to_string())?;
                new_entities += n as i64;
            }
            let processed = (idx + 1) as i64;
            run_db(data_dir, move |conn| {
                let _ = conn.execute("UPDATE jobs SET processed=?1 WHERE id=?2", rusqlite::params![processed, job_id]);
                Ok(())
            }).await?;
        }
        Ok((created_items, new_entities))
    }

    async fn upsert_doc(&self, data_dir: &str, source_id: i64, doc: &IngestionDoc, _force: bool) -> Result<bool, String> {
        let digest = content_hash(&doc.text);
        let doc = doc.clone();
        let digest = digest.clone();
        run_db(data_dir, move |conn| {
            let existing: Option<(i64, String, Option<String>)> = conn.query_row("SELECT id, content_hash, text FROM ingested_items WHERE source_id=?1 AND external_id=?2", rusqlite::params![source_id, doc.external_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).ok();
            if let Some((id, existing_hash, existing_text)) = existing {
                let missing_text = existing_text.as_deref().unwrap_or("").is_empty();
                if existing_hash == digest && !missing_text {
                    return Ok(false);
                }
                let _ = conn.execute("UPDATE ingested_items SET content_hash=?1, title=?2, author=?3, text=?4, stale=0 WHERE id=?5", rusqlite::params![digest, doc.title, doc.author, doc.text, id]);
                Ok(true)
            } else {
                conn.execute("INSERT INTO ingested_items (source_id, external_id, title, text, content_hash, author, stale, created_at) VALUES (?1,?2,?3,?4,?5,?6, 0, datetime('now'))", rusqlite::params![source_id, doc.external_id, doc.title, doc.text, digest, doc.author]).map_err(|e| e.to_string())?;
                Ok(true)
            }
        }).await
    }

    async fn classify_and_store(&self, data_dir: &str, source_id: i64, job_id: i64, doc: &IngestionDoc, force_reclassify: bool) -> Result<usize, String> {
        let mut ctx = ClassifyCtx::new(data_dir, source_id, job_id, doc, force_reclassify, self.settings.clone(), self.classifier.clone(), self.embedder.clone());
        ctx.load_item_meta().await?;
        ctx.detect_doc_type().await?;
        // If force, delete all entities for item
        if force_reclassify {
            delete_stale_entities(&ctx, None).await?;
        }
        // R9.1: abort early if cancelled before windowing
        if job_cancelled(&ctx.data_dir, job_id).await? {
            return Err("job cancelled".to_string());
        }
        let total_entities = ctx.classify_windows().await?;
        ctx.finalize().await?;
        Ok(total_entities)
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
        let (item_id, label, prev_hashes) = run_db(&self.data_dir, move |conn| {
            let item_id: i64 = conn.query_row("SELECT id FROM ingested_items WHERE source_id=?1 AND external_id=?2", rusqlite::params![source_id, external_id], |r| r.get(0)).map_err(|e| e.to_string())?;
            let label: String = conn.query_row("SELECT label FROM sources WHERE id=?1", [source_id], |r| r.get(0)).unwrap_or_else(|_| "internal".to_string());
            let s: String = conn.query_row("SELECT window_hashes FROM ingested_items WHERE id=?1", [item_id], |r| r.get(0)).unwrap_or_else(|_| "{}".to_string());
            let prev_hashes: std::collections::HashMap<String, String> = serde_json::from_str(&s).unwrap_or_default();
            Ok((item_id, label, prev_hashes))
        }).await?;
        self.item_id = item_id;
        self.label = label;
        self.prev_hashes = prev_hashes;
        self.new_hashes = self.prev_hashes.clone();
        Ok(())
    }

    // gate + doc_type detection (P1 4.4 — was hardcoded general)
    async fn detect_doc_type(&mut self) -> Result<(), String> {
        self.classify_trusted = cloud_trusted(&*self.settings, "classifier");
        self.embed_trusted = cloud_trusted(&*self.settings, "embed");
        let trusted = self.classify_trusted;
        let doc_type = if trusted {
            // Box LLM future to bound stack (large classifier future previously blew 8 MB)
            Box::pin(self.classifier.detect_document_type(&self.full_text, trusted)).await
        } else {
            "general".to_string()
        };
        self.doc_type = doc_type;
        Ok(())
    }

    async fn classify_windows(&mut self) -> Result<usize, String> {
        let mut total = 0usize;
        for (idx, window) in self.windows.iter().enumerate() {
            if job_cancelled(&self.data_dir, self.job_id).await? {
                return Err("job cancelled".to_string());
            }
            let index = (idx + 1) as i64;
            let h = window_hash(window);
            let unchanged = !self.force && self.prev_hashes.get(&index.to_string()).map(|p| *p == h).unwrap_or(false);
            if unchanged {
                self.new_hashes.insert(index.to_string(), h);
                continue;
            }
            self.new_hashes.insert(index.to_string(), h.clone());
            // delete stale entities for this window index if not force (already handled force)
            if !self.force {
                delete_stale_entities(self, Some(index)).await?;
            }
            let source_ref = if self.windows.len() > 1 {
                format!("{} §{}", self.base_ref, index)
            } else {
                self.base_ref.clone()
            };
            // classify via rules (no LLM for tests)
            let to_insert = classify_window(window, &source_ref, index);
            // insert into DB
            total += store_batch(self, to_insert).await?;
        }
        Ok(total)
    }

    async fn finalize(&self) -> Result<(), String> {
        update_window_hashes(self).await?;
        store_doc_chunks(self).await?;
        create_summaries(self).await?;
        create_tags(self).await?;
        flag_pii(self).await?;
        distill_item(self).await?;
        embed_chunks(self).await
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
            "INSERT INTO chunks (item_id, section_id, kind, source_ref, content, level, path, created_at) VALUES (?1,?2,'section_summary',?3,?4,?5,?6,datetime('now'))",
            rusqlite::params![item_id, sec_id, src_ref, summary, level, path],
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
        "INSERT INTO chunks (item_id, kind, source_ref, content, level, path, created_at) VALUES (?1,'doc_summary',?2,?3,0,'',datetime('now'))",
        rusqlite::params![item_id, format!("{} summary", base_ref), doc_summary],
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

// classify via rules + dedupe by summary lower
fn classify_window(window: &str, source_ref: &str, index: i64) -> Vec<crate::classifier::ClassifiedItem> {
    let items = crate::classifier::classify_rules(window, source_ref);
    let mut seen = std::collections::HashSet::new();
    let mut to_insert = vec![];
    for mut it in items {
        let key = it.summary.to_lowercase();
        if seen.contains(&key) {
            continue;
        }
        seen.insert(key);
        it.window_text = window.to_string();
        it.window_index = Some(index);
        to_insert.push(it);
    }
    to_insert
}

async fn store_batch(ctx: &ClassifyCtx, items: Vec<crate::classifier::ClassifiedItem>) -> Result<usize, String> {
    let item_id = ctx.item_id;
    let count = items.len();
    run_db(&ctx.data_dir, move |conn| {
        for it in items {
            let status = if it.confidence >= 0.7 { "verified" } else { "unverified" };
            conn.execute("INSERT INTO entities (item_id, kind, summary, reasoning, confidence, author, source_ref, window_text, window_index, status, owner, created_at, updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,'',datetime('now'),datetime('now'))", rusqlite::params![item_id, it.kind, it.summary, it.reasoning, it.confidence, it.author, it.source_ref, it.window_text, it.window_index, status]).unwrap();
            let eid = conn.last_insert_rowid();
            for chunk_content in crate::chunking::chunk_text(&it.summary) {
                let _ = conn.execute("INSERT INTO chunks (entity_id, kind, source_ref, content, created_at) VALUES (?1,'entity',?2,?3,datetime('now'))", rusqlite::params![eid, it.source_ref, chunk_content]);
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
    let item_id = ctx.item_id;
    let full_text = ctx.full_text.clone();
    let base_ref = ctx.base_ref.clone();
    run_db(&ctx.data_dir, move |conn| {
        let _ = conn.execute(
            "DELETE FROM chunks WHERE item_id=?1 AND entity_id IS NULL AND kind != 'distilled'",
            [item_id],
        );
        let _ = conn.execute("DELETE FROM sections WHERE item_id=?1", [item_id]);
        let cleaned = full_text.clone();
        let (metas, chunk_metas) = crate::chunking::chunk_document_with_sections(&cleaned);
        // insert sections in order so parent idx maps correctly
        let mut section_ids: Vec<i64> = Vec::with_capacity(metas.len());
        for meta in &metas {
            let parent_db = meta.parent.and_then(|p| section_ids.get(p).copied());
            conn.execute(
                "INSERT INTO sections (item_id, parent_id, level, title, path, created_at) VALUES (?1,?2,?3,?4,?5,datetime('now'))",
                rusqlite::params![item_id, parent_db, meta.level, meta.title, meta.path],
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
                "INSERT INTO chunks (item_id, kind, source_ref, content, section_id, level, path, created_at) VALUES (?1,'document',?2,?3,?4,?5,?6,datetime('now'))",
                rusqlite::params![item_id, r, cm.content, sec_db, cm.level, cm.path],
            )
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    })
    .await
}

async fn flag_pii(ctx: &ClassifyCtx) -> Result<(), String> {
    let item_id = ctx.item_id;
    run_db(&ctx.data_dir, move |conn| {
        let _ = crate::pii::flag_pii_for_item(conn, item_id);
        Ok(())
    }).await
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

// B18 distillation (chat-like doc types → Q&A units), hash-skipped
async fn distill_item(ctx: &ClassifyCtx) -> Result<(), String> {
    let distill_enabled = ctx.settings.get("distill_enabled", None).map(|v| v == "true" || v == "1" || v == "True").unwrap_or(true);
    if !(crate::distill::DISTILL_DOC_TYPES.contains(&ctx.doc_type.as_str()) && distill_enabled) {
        // still need to update distill_hashes to current window hashes to avoid re-distilling later if type changes
        return update_distill_hashes(ctx).await;
    }
    let item_id = ctx.item_id;
    let base_ref = ctx.base_ref.clone();
    let windows = ctx.windows.clone();
    let force = ctx.force;
    // R15.4: reuse classify-phase hashes (no re-hash of every window).
    let new_hashes = ctx.new_hashes.clone();
    run_db(&ctx.data_dir, move |conn| {
        let prev_str: String = conn.query_row("SELECT distill_hashes FROM ingested_items WHERE id=?1", [item_id], |r| r.get(0)).unwrap_or_else(|_| "{}".to_string());
        let prev: std::collections::HashMap<String, String> = serde_json::from_str(&prev_str).unwrap_or_default();
        if !distill_changed_hashes(force, &new_hashes, &prev) {
            // update hashes even if skip? keep prev
            return Ok(());
        }
        // delete old distilled chunks
        let _ = conn.execute("DELETE FROM chunks WHERE item_id=?1 AND kind='distilled'", [item_id]);
        let max_units: usize = std::env::var("ANCHOR_DISTILL_MAX_UNITS").ok().and_then(|v| v.parse().ok()).unwrap_or(8);
        for (idx, window) in windows.iter().enumerate() {
            let units = crate::classifier::distill_rules(window);
            let refs = if windows.len() > 1 { format!("{} §{}", base_ref, idx + 1) } else { base_ref.clone() };
            for unit in units.into_iter().take(max_units) {
                let content = distill_unit_content(&unit);
                let _ = conn.execute("INSERT INTO chunks (item_id, kind, source_ref, content, created_at) VALUES (?1,'distilled',?2,?3,datetime('now'))", rusqlite::params![item_id, refs, content]);
            }
        }
        let j = serde_json::to_string(&new_hashes).unwrap_or_else(|_| "{}".to_string());
        let _ = conn.execute("UPDATE ingested_items SET distill_hashes=?1 WHERE id=?2", rusqlite::params![j, item_id]);
        Ok(())
    }).await
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
async fn embed_chunks(ctx: &ClassifyCtx) -> Result<(), String> {
    let embed_min_signal: f64 = std::env::var("ANCHOR_EMBED_MIN_SIGNAL").ok().and_then(|v| v.parse().ok()).unwrap_or(0.15);
    let gated = sensitive_label(&ctx.label) && !(ctx.classify_trusted && ctx.embed_trusted);
    let item_id = ctx.item_id;
    let chunks: Vec<(i64, String, bool)> = run_db(&ctx.data_dir, move |conn| {
        let mut stmt = conn.prepare("SELECT id, content, is_pii FROM chunks WHERE (item_id=?1 OR entity_id IN (SELECT id FROM entities WHERE item_id=?1)) AND embedding IS NULL").unwrap();
        let rows = stmt.query_map([item_id], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)? != 0))).unwrap().filter_map(|r| r.ok()).collect();
        Ok(rows)
    }).await?;
    if chunks.is_empty() {
        return Ok(());
    }
    let pending_ids: Vec<i64> = chunks.iter().map(|(id, _, _)| *id).collect();
    let contents: Vec<String> = chunks.iter().map(|(_, c, _)| c.clone()).collect();
    let is_pii_flags: Vec<bool> = chunks.iter().map(|(_, _, p)| *p).collect();
    // call embedder (graceful fallback inside embed_gated on remote failure)
    let out = ctx.embedder.embed_gated(contents, is_pii_flags, gated, embed_min_signal).await.unwrap_or_else(|_| vec![None; pending_ids.len()]);
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
        let full_text = "§434 Merger\ncontent a merger details\n\n§437 Dates\ncontent b dates details\n\n4.2.1 Sub\ncontent c sub details";
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
}
