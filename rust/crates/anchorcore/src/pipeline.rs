//! Ingestion pipeline — port of `backend/app/pipeline.py:1`.
//! Folder → chunk → classify → distill → pii → embed (rule-based, no LLM needed for tests).

use std::sync::Arc;

use rusqlite::Connection;

use crate::classifier::Classifier;
use crate::connectors::{folder::FolderConnector, IngestionDoc};
use crate::embedder::Embedder;
use crate::settings::SettingsService;

fn content_hash(text: &str) -> String {
    crate::hashing::content_hash(text)
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
    let is_local = base.starts_with("http://localhost") || base.starts_with("http://127.0.0.1");
    if is_local { true } else { std::env::var("ANCHOR_CLOUD_TRUST").as_deref() == Ok("1") }
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
fn is_cancelled(data_dir: &str, job_id: i64) -> bool {
    let db_path = crate::db::resolve_db_path(data_dir);
    let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| Connection::open(&db_path).unwrap());
    let status: String = conn.query_row("SELECT status FROM jobs WHERE id=?1", [job_id], |r| r.get(0)).unwrap_or_else(|_| "running".to_string());
    status == "cancelled"
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
        let settings = self.settings.clone();
        let classifier = self.classifier.clone();
        let embedder = self.embedder.clone();
        // R10.4: respect bounded concurrency — do NOT flip pending→running here
        // JobManager::maybe_promote is the sole owner of pending→running.
        // If this job is pending, wait for a slot (poll DB until promoted or cancelled).
        {
            let db_path = crate::db::resolve_db_path(&data_dir);
            let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| Connection::open(&db_path).unwrap());
            let status: String = conn.query_row("SELECT status FROM jobs WHERE id=?1", [job_id], |r| r.get(0)).unwrap_or_else(|_| "pending".to_string());
            if status == "cancelled" {
                self.jobs.maybe_promote(&conn);
                return;
            }
            if status == "pending" {
                drop(conn);
                // wait for promotion (MAX_CONCURRENT gate)
                loop {
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                    let db_path2 = crate::db::resolve_db_path(&data_dir);
                    let conn2 = crate::db::init_db(&db_path2).unwrap_or_else(|_| Connection::open(&db_path2).unwrap());
                    let cur: String = conn2.query_row("SELECT status FROM jobs WHERE id=?1", [job_id], |r| r.get(0)).unwrap_or_else(|_| "pending".to_string());
                    if cur == "cancelled" {
                        crate::jobs::JobManager::new().maybe_promote(&conn2);
                        return;
                    }
                    if cur == "running" {
                        break;
                    }
                    if cur != "pending" {
                        // failed/done — should not happen for pending waiter, just exit
                        return;
                    }
                }
            } else if status != "running" {
                // unexpected state (failed/done) — nothing to do
                return;
            }
            // job is now running (promoted by JobManager); proceed without extra UPDATE
        }
        let res = Box::pin(self.run_sync_inner(source_id, job_id, force_reclassify, &data_dir, settings.clone(), classifier.clone(), embedder.clone())).await;
        let db_path = crate::db::resolve_db_path(&data_dir);
        let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| Connection::open(&db_path).unwrap());
        match res {
            Ok((items, entities)) => {
                // R9.1: do not clobber concurrent cancel
                let cur: String = conn.query_row("SELECT status FROM jobs WHERE id=?1", [job_id], |r| r.get(0)).unwrap_or_else(|_| "done".to_string());
                if cur == "cancelled" {
                    self.jobs.maybe_promote(&conn);
                } else {
                    let result = serde_json::json!({"items": items, "entities": entities}).to_string();
                    let _ = conn.execute("UPDATE jobs SET status='done', finished_at=datetime('now'), result=?1, error=NULL WHERE id=?2", rusqlite::params![result, job_id]);
                    record_sync_success(&conn, source_id);
                    // update job total/processed
                    let _ = conn.execute("UPDATE jobs SET total=?1, processed=?1 WHERE id=?2", rusqlite::params![items, job_id]);
                    // promote next pending
                    self.jobs.maybe_promote(&conn);
                }
            }
            Err(e) if e == "job cancelled" => {
                // R9.1: cancelled must stay cancelled, no error_count bump, promote next
                let _ = conn.execute("UPDATE jobs SET status='cancelled', finished_at=datetime('now'), error='job cancelled' WHERE id=?1", [job_id]);
                self.jobs.maybe_promote(&conn);
            }
            Err(e) => {
                // do not clobber a concurrent cancel that already set cancelled
                let cur: String = conn.query_row("SELECT status FROM jobs WHERE id=?1", [job_id], |r| r.get(0)).unwrap_or_else(|_| "failed".to_string());
                if cur == "cancelled" {
                    self.jobs.maybe_promote(&conn);
                } else {
                    let _ = conn.execute("UPDATE jobs SET status='failed', finished_at=datetime('now'), error=?1 WHERE id=?2", rusqlite::params![e, job_id]);
                    record_sync_error(&conn, source_id, &e);
                    self.jobs.maybe_promote(&conn);
                }
            }
        }
    }

    async fn run_sync_inner(
        &self,
        source_id: i64,
        job_id: i64,
        force_reclassify: bool,
        data_dir: &str,
        settings: Arc<SettingsService>,
        classifier: Arc<Classifier>,
        embedder: Arc<Embedder>,
    ) -> Result<(i64, i64), String> {
        // load source
        let (connector_type, config_str, _label) = {
            let db_path = crate::db::resolve_db_path(data_dir);
            let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| Connection::open(&db_path).unwrap());
            let mut stmt = conn.prepare("SELECT connector, config, label FROM sources WHERE id = ?1").map_err(|e| e.to_string())?;
            let row = stmt.query_row([source_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))).map_err(|_| "Source not found".to_string())?;
            row
        };
        let config: serde_json::Value = serde_json::from_str(&config_str).unwrap_or(serde_json::json!({}));
        // fetch docs — R10.2: Jira/GDrive must not silently succeed with 0 items
        let docs: Vec<IngestionDoc> = match connector_type.as_str() {
            "folder" => {
                let fc = FolderConnector::new(&config).map_err(|e| e.to_string())?;
                let (docs, _cursor) = fc.fetch().map_err(|e| e.to_string())?;
                docs
            }
            "jira" => {
                return Err("Jira ingestion not yet wired in Rust pipeline: configure folder connector or implement JiraConnector::fetch".to_string());
            }
            "gdrive" => {
                return Err("GDrive ingestion not yet wired in Rust pipeline: configure folder connector or implement GDriveConnector::fetch".to_string());
            }
            _ => {
                return Err(format!("unknown connector '{}'", connector_type));
            }
        };
        // update job total
        {
            let db_path = crate::db::resolve_db_path(data_dir);
            let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| Connection::open(&db_path).unwrap());
            let _ = conn.execute("UPDATE jobs SET total=?1 WHERE id=?2", rusqlite::params![docs.len() as i64, job_id]);
        }
        let mut created_items = 0i64;
        let mut new_entities = 0i64;
        let mut processed = 0i64;
        for doc in docs {
            // check if job was cancelled
            {
                let db_path = crate::db::resolve_db_path(data_dir);
                let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| Connection::open(&db_path).unwrap());
                let status: String = conn.query_row("SELECT status FROM jobs WHERE id=?1", [job_id], |r| r.get(0)).unwrap_or_else(|_| "running".to_string());
                if status == "cancelled" {
                    return Err("job cancelled".to_string());
                }
            }
            let upserted = Box::pin(self.upsert_doc(data_dir, source_id, &doc, force_reclassify)).await.map_err(|e| e.to_string())?;
            if upserted || force_reclassify {
                created_items += 1;
                let n = Box::pin(self.classify_and_store(data_dir, source_id, job_id, &doc, force_reclassify, settings.clone(), classifier.clone(), embedder.clone())).await.map_err(|e| e.to_string())?;
                new_entities += n as i64;
            }
            processed += 1;
            let db_path = crate::db::resolve_db_path(data_dir);
            let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| Connection::open(&db_path).unwrap());
            let _ = conn.execute("UPDATE jobs SET processed=?1 WHERE id=?2", rusqlite::params![processed, job_id]);
        }
        Ok((created_items, new_entities))
    }

    async fn upsert_doc(&self, data_dir: &str, source_id: i64, doc: &IngestionDoc, _force: bool) -> Result<bool, String> {
        let digest = content_hash(&doc.text);
        let result = tokio::task::spawn_blocking({
            let data_dir = data_dir.to_string();
            let doc = doc.clone();
            let digest = digest.clone();
            move || {
                let db_path = crate::db::resolve_db_path(&data_dir);
                let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| Connection::open(&db_path).unwrap());
                let existing: Option<(i64, String, Option<String>)> = conn.query_row("SELECT id, content_hash, text FROM ingested_items WHERE source_id=?1 AND external_id=?2", rusqlite::params![source_id, doc.external_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).ok();
                if let Some((id, existing_hash, existing_text)) = existing {
                    let missing_text = existing_text.as_deref().unwrap_or("").is_empty();
                    if existing_hash == digest && !missing_text {
                        return Ok::<bool, String>(false);
                    }
                    let _ = conn.execute("UPDATE ingested_items SET content_hash=?1, title=?2, author=?3, text=?4, stale=0 WHERE id=?5", rusqlite::params![digest, doc.title, doc.author, doc.text, id]);
                    Ok(true)
                } else {
                    conn.execute("INSERT INTO ingested_items (source_id, external_id, title, text, content_hash, author, stale, created_at) VALUES (?1,?2,?3,?4,?5,?6, 0, datetime('now'))", rusqlite::params![source_id, doc.external_id, doc.title, doc.text, digest, doc.author]).map_err(|e| e.to_string())?;
                    Ok(true)
                }
            }
        }).await.map_err(|e| e.to_string())?;
        result
    }

    async fn classify_and_store(
        &self,
        data_dir: &str,
        source_id: i64,
        job_id: i64,
        doc: &IngestionDoc,
        force_reclassify: bool,
        settings: Arc<SettingsService>,
        classifier: Arc<Classifier>,
        embedder: Arc<Embedder>,
    ) -> Result<usize, String> {
        let base_ref = if doc.source_ref.is_empty() { doc.title.clone() } else { doc.source_ref.clone() };
        let windows = crate::chunking::classify_windows(&doc.text);
        // load item
        let item_id: i64 = tokio::task::spawn_blocking({
            let data_dir = data_dir.to_string();
            let external_id = doc.external_id.clone();
            move || {
                let db_path = crate::db::resolve_db_path(&data_dir);
                let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| Connection::open(&db_path).unwrap());
                conn.query_row("SELECT id FROM ingested_items WHERE source_id=?1 AND external_id=?2", rusqlite::params![source_id, external_id], |r| r.get(0)).unwrap()
            }
        }).await.map_err(|e| e.to_string())?;
        // gate + doc_type detection (P1 4.4 — was hardcoded general)
        let classify_trusted = cloud_trusted(&*settings, "classifier");
        let embed_trusted = cloud_trusted(&*settings, "embed");
        let doc_type: String = if classify_trusted {
            // Box LLM future to bound stack (large classifier future previously blew 8 MB)
            Box::pin(classifier.detect_document_type(&doc.text, classify_trusted)).await
        } else {
            "general".to_string()
        };
        let label: String = tokio::task::spawn_blocking({
            let data_dir = data_dir.to_string();
            move || {
                let db_path = crate::db::resolve_db_path(&data_dir);
                let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| Connection::open(&db_path).unwrap());
                conn.query_row("SELECT label FROM sources WHERE id=?1", [source_id], |r| r.get(0)).unwrap_or_else(|_| "internal".to_string())
            }
        }).await.map_err(|e| e.to_string())?;
        let _gated = sensitive_label(&label) && !(classify_trusted && embed_trusted);
        // For simplicity, we ignore gated and distill for folder tests; distill only for meeting etc
        // Handle window hashes for skip
        let prev_hashes: std::collections::HashMap<String, String> = tokio::task::spawn_blocking({
            let data_dir = data_dir.to_string();
            move || {
                let db_path = crate::db::resolve_db_path(&data_dir);
                let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| Connection::open(&db_path).unwrap());
                let s: String = conn.query_row("SELECT window_hashes FROM ingested_items WHERE id=?1", [item_id], |r| r.get(0)).unwrap_or_else(|_| "{}".to_string());
                serde_json::from_str::<std::collections::HashMap<String,String>>(&s).unwrap_or_default()
            }
        }).await.map_err(|e| e.to_string())?;
        let mut new_hashes = prev_hashes.clone();
        let mut total_entities = 0usize;
        // If force, delete all entities for item
        if force_reclassify {
            tokio::task::spawn_blocking({
                let data_dir = data_dir.to_string();
                move || {
                    let db_path = crate::db::resolve_db_path(&data_dir);
                    let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| Connection::open(&db_path).unwrap());
                    let ids: Vec<i64> = conn.prepare("SELECT id FROM entities WHERE item_id=?1").unwrap().query_map([item_id], |r| r.get(0)).unwrap().filter_map(|r| r.ok()).collect();
                    for eid in ids {
                        let _ = conn.execute("DELETE FROM relationships WHERE from_entity_id=?1 OR to_entity_id=?1", [eid]);
                        let _ = conn.execute("DELETE FROM merge_actions WHERE entity_a_id=?1 OR entity_b_id=?1", [eid]);
                    }
                    let _ = conn.execute("DELETE FROM entities WHERE item_id=?1", [item_id]);
                }
            }).await.map_err(|e| e.to_string())?;
        }
        // R9.1: abort early if cancelled before windowing
        if is_cancelled(data_dir, job_id) {
            return Err("job cancelled".to_string());
        }
        // classify each window
        for (idx, window) in windows.iter().enumerate() {
            if is_cancelled(data_dir, job_id) {
                return Err("job cancelled".to_string());
            }
            let index = (idx + 1) as i64;
            let h = window_hash(window);
            if !force_reclassify {
                if let Some(prev) = prev_hashes.get(&index.to_string()) {
                    if *prev == h {
                        new_hashes.insert(index.to_string(), h);
                        continue;
                    }
                }
            }
            new_hashes.insert(index.to_string(), h.clone());
            // delete stale entities for this window index if not force (already handled force)
            if !force_reclassify {
                tokio::task::spawn_blocking({
                    let data_dir = data_dir.to_string();
                    move || {
                        let db_path = crate::db::resolve_db_path(&data_dir);
                        let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| Connection::open(&db_path).unwrap());
                        let ids: Vec<i64> = conn.prepare("SELECT id FROM entities WHERE item_id=?1 AND window_index=?2").unwrap().query_map(rusqlite::params![item_id, index], |r| r.get(0)).unwrap().filter_map(|r| r.ok()).collect();
                        for eid in ids {
                            let _ = conn.execute("DELETE FROM relationships WHERE from_entity_id=?1 OR to_entity_id=?1", [eid]);
                            let _ = conn.execute("DELETE FROM merge_actions WHERE entity_a_id=?1 OR entity_b_id=?1", [eid]);
                        }
                        let _ = conn.execute("DELETE FROM entities WHERE item_id=?1 AND window_index=?2", rusqlite::params![item_id, index]);
                    }
                }).await.map_err(|e| e.to_string())?;
            }
            let source_ref = if windows.len() > 1 { format!("{} §{}", base_ref, index) } else { base_ref.clone() };
            // classify via rules (no LLM for tests)
            let items = crate::classifier::classify_rules(window, &source_ref);
            // dedupe by summary lower
            let mut seen = std::collections::HashSet::new();
            let mut to_insert = vec![];
            for mut it in items {
                let key = it.summary.to_lowercase();
                if seen.contains(&key) { continue; }
                seen.insert(key);
                it.window_text = window.clone();
                it.window_index = Some(index);
                to_insert.push(it);
            }
            // insert into DB
            let data_dir_clone = data_dir.to_string();
            let _window_clone = window.clone();
            tokio::task::spawn_blocking(move || {
                let db_path = crate::db::resolve_db_path(&data_dir_clone);
                let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| Connection::open(&db_path).unwrap());
                for it in to_insert {
                    let status = if it.confidence >= 0.7 { "verified" } else { "unverified" };
                    conn.execute("INSERT INTO entities (item_id, kind, summary, reasoning, confidence, author, source_ref, window_text, window_index, status, owner, created_at, updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,'',datetime('now'),datetime('now'))", rusqlite::params![item_id, it.kind, it.summary, it.reasoning, it.confidence, it.author, it.source_ref, it.window_text, it.window_index, status]).unwrap();
                    let eid = conn.last_insert_rowid();
                    for chunk_content in crate::chunking::chunk_text(&it.summary) {
                        let _ = conn.execute("INSERT INTO chunks (entity_id, kind, source_ref, content, created_at) VALUES (?1,'entity',?2,?3,datetime('now'))", rusqlite::params![eid, it.source_ref, chunk_content]);
                    }
                }
            }).await.map_err(|e| e.to_string())?;
            total_entities += seen.len();
        }
        // update window_hashes
        tokio::task::spawn_blocking({
            let data_dir = data_dir.to_string();
            move || {
                let db_path = crate::db::resolve_db_path(&data_dir);
                let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| Connection::open(&db_path).unwrap());
                let j = serde_json::to_string(&new_hashes).unwrap();
                let _ = conn.execute("UPDATE ingested_items SET window_hashes=?1 WHERE id=?2", rusqlite::params![j, item_id]);
            }
        }).await.map_err(|e| e.to_string())?;
        // full-document chunks
        tokio::task::spawn_blocking({
            let data_dir = data_dir.to_string();
            let full_text = doc.text.clone();
            let base_ref = base_ref.clone();
            move || {
                let db_path = crate::db::resolve_db_path(&data_dir);
                let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| Connection::open(&db_path).unwrap());
                let _ = conn.execute("DELETE FROM chunks WHERE item_id=?1 AND entity_id IS NULL AND kind != 'distilled'", [item_id]);
                let cleaned = full_text.clone(); // no cleaning for now
                let doc_chunks = crate::chunking::chunk_document(&cleaned);
                for (idx, chunk_content) in doc_chunks.iter().enumerate() {
                    let r = if doc_chunks.len() > 1 { format!("{} §{}", base_ref, idx+1) } else { base_ref.clone() };
                    let _ = conn.execute("INSERT INTO chunks (item_id, kind, source_ref, content, created_at) VALUES (?1,'document',?2,?3,datetime('now'))", rusqlite::params![item_id, r, chunk_content]);
                }
            }
        }).await.map_err(|e| e.to_string())?;
        // flag pii
        tokio::task::spawn_blocking({
            let data_dir = data_dir.to_string();
            move || {
                let db_path = crate::db::resolve_db_path(&data_dir);
                let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| Connection::open(&db_path).unwrap());
                let _ = crate::pii::flag_pii_for_item(&conn, item_id);
            }
        }).await.map_err(|e| e.to_string())?;
        // B18 distillation (chat-like doc types → Q&A units), hash-skipped
        let do_distill = {
            let distill_enabled = settings.get("distill_enabled", None).map(|v| v=="true" || v=="1" || v=="True").unwrap_or(true);
            crate::distill::DISTILL_DOC_TYPES.contains(&doc_type.as_str()) && distill_enabled
        };
        if do_distill {
            let distill_result = tokio::task::spawn_blocking({
                let data_dir = data_dir.to_string();
                let _doc_text = doc.text.clone();
                let base_ref_clone = base_ref.clone();
                let windows_clone = windows.clone();
                let force = force_reclassify;
                move || {
                    let db_path = crate::db::resolve_db_path(&data_dir);
                    let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| Connection::open(&db_path).unwrap());
                    let prev_str: String = conn.query_row("SELECT distill_hashes FROM ingested_items WHERE id=?1", [item_id], |r| r.get(0)).unwrap_or_else(|_| "{}".to_string());
                    let prev: std::collections::HashMap<String,String> = serde_json::from_str(&prev_str).unwrap_or_default();
                    let mut new_hashes = prev.clone();
                    // hash skip check: if all windows hashes equal prev and not force, skip entirely
                    let mut should_run = force;
                    if !force {
                        for (idx, w) in windows_clone.iter().enumerate() {
                            let h = crate::hashing::window_hash(w);
                            if prev.get(&(idx+1).to_string()).map(|v| v != &h).unwrap_or(true) {
                                should_run = true;
                                break;
                            }
                        }
                    }
                    if !should_run {
                        // update hashes even if skip? keep prev
                        return Ok::<(), String>(());
                    }
                    // delete old distilled chunks
                    let _ = conn.execute("DELETE FROM chunks WHERE item_id=?1 AND kind='distilled'", [item_id]);
                    let max_units: usize = std::env::var("ANCHOR_DISTILL_MAX_UNITS").ok().and_then(|v| v.parse().ok()).unwrap_or(8);
                    for (idx, window) in windows_clone.iter().enumerate() {
                        let h = crate::hashing::window_hash(window);
                        new_hashes.insert((idx+1).to_string(), h);
                        let units = crate::classifier::distill_rules(window);
                        let refs = if windows_clone.len() > 1 { format!("{} §{}", base_ref_clone, idx+1) } else { base_ref_clone.clone() };
                        for unit in units.into_iter().take(max_units) {
                            let mut content = format!("Q: {}\nA: {}", unit.question, unit.answer);
                            if !unit.terms.is_empty() {
                                content.push_str(&format!("\nTerms: {}", unit.terms.join(", ")));
                            }
                            if !unit.systems.is_empty() {
                                content.push_str(&format!("\nSystems: {}", unit.systems.join(", ")));
                            }
                            let _ = conn.execute("INSERT INTO chunks (item_id, kind, source_ref, content, created_at) VALUES (?1,'distilled',?2,?3,datetime('now'))", rusqlite::params![item_id, refs, content]);
                        }
                    }
                    let j = serde_json::to_string(&new_hashes).unwrap_or_else(|_| "{}".to_string());
                    let _ = conn.execute("UPDATE ingested_items SET distill_hashes=?1 WHERE id=?2", rusqlite::params![j, item_id]);
                    Ok(())
                }
            }).await.map_err(|e| e.to_string())?;
            distill_result.map_err(|e| e.to_string())?;
        } else {
            // still need to update distill_hashes to current window hashes to avoid re-distilling later if type changes
            let _ = tokio::task::spawn_blocking({
                let data_dir = data_dir.to_string();
                let windows_clone = windows.clone();
                move || {
                    let db_path = crate::db::resolve_db_path(&data_dir);
                    if let Ok(conn) = crate::db::init_db(&db_path) {
                        let mut map = std::collections::HashMap::new();
                        for (idx, w) in windows_clone.iter().enumerate() {
                            map.insert((idx+1).to_string(), crate::hashing::window_hash(w));
                        }
                        if let Ok(j) = serde_json::to_string(&map) {
                            let _ = conn.execute("UPDATE ingested_items SET distill_hashes=?1 WHERE id=?2", rusqlite::params![j, item_id]);
                        }
                    }
                }
            }).await;
        }
        // embed (gated) — wire embedder.embed_gated + sync_vec (P0 3.2)
        let embed_min_signal: f64 = std::env::var("ANCHOR_EMBED_MIN_SIGNAL").ok().and_then(|v| v.parse().ok()).unwrap_or(0.15);
        let gated = sensitive_label(&label) && !(classify_trusted && embed_trusted);
        let chunks: Vec<(i64, String, bool)> = tokio::task::spawn_blocking({
            let data_dir = data_dir.to_string();
            move || {
                let db_path = crate::db::resolve_db_path(&data_dir);
                let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| Connection::open(&db_path).unwrap());
                let mut stmt = conn.prepare("SELECT id, content, is_pii FROM chunks WHERE (item_id=?1 OR entity_id IN (SELECT id FROM entities WHERE item_id=?1)) AND embedding IS NULL").unwrap();
                let rows = stmt.query_map([item_id], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)? != 0))).unwrap().filter_map(|r| r.ok()).collect();
                rows
            }
        }).await.map_err(|e| e.to_string())?;
        if !chunks.is_empty() {
            let pending_ids: Vec<i64> = chunks.iter().map(|(id, _, _)| *id).collect();
            let contents: Vec<String> = chunks.iter().map(|(_, c, _)| c.clone()).collect();
            let is_pii_flags: Vec<bool> = chunks.iter().map(|(_, _, p)| *p).collect();
            // call embedder (graceful fallback inside embed_gated on remote failure)
            let out = embedder.embed_gated(contents, is_pii_flags, gated, embed_min_signal).await.unwrap_or_else(|_| vec![None; pending_ids.len()]);
            for (idx, blob_opt) in out.into_iter().enumerate() {
                if let Some(blob) = blob_opt {
                    let cid = pending_ids[idx];
                    let data_dir_c = data_dir.to_string();
                    let blob_c = blob.clone();
                    let embedder_c = embedder.clone();
                    // update DB + vec0 sync in blocking task
                    let _ = tokio::task::spawn_blocking(move || {
                        let db_path = crate::db::resolve_db_path(&data_dir_c);
                        if let Ok(conn) = crate::db::init_db(&db_path) {
                            let _ = conn.execute("UPDATE chunks SET embedding=?1 WHERE id=?2", rusqlite::params![blob_c, cid]);
                            let _ = embedder_c.sync_vec(&conn, cid, &blob_c);
                        }
                    }).await;
                }
            }
        }

        Ok(total_entities)
    }
}
