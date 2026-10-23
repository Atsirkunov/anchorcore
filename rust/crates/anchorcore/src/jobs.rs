//! JobManager (R4.1) — bounded concurrency MAX_CONCURRENT=2, pending queue.
//! Port of `backend/app/jobs.py` / `JobManager`.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::health::AppState;

pub const MAX_CONCURRENT: usize = 2;

static PROMOTE_LOCK: Mutex<()> = Mutex::new(());

/// R30.1: why `create_job` can fail without a DB error.
#[derive(Debug)]
pub enum CreateJobError {
    /// A sync-family job is already pending/running for this source — carries its id.
    Conflict(i64),
    Db(rusqlite::Error),
}

impl From<rusqlite::Error> for CreateJobError {
    fn from(e: rusqlite::Error) -> Self {
        CreateJobError::Db(e)
    }
}

impl std::fmt::Display for CreateJobError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CreateJobError::Conflict(id) => write!(f, "a sync job is already active for this source (job {})", id),
            CreateJobError::Db(e) => write!(f, "{}", e),
        }
    }
}

/// Sync-family kinds share the ingest write path: at most one may be
/// pending/running per source (R30.1). `embed` jobs only fill vectors and
/// are not guarded.
fn is_sync_family(kind: &str) -> bool {
    matches!(kind, "sync" | "reclassify")
}

/// Id of the active (pending/running) sync-family job for a source, if any.
fn active_sync_job(conn: &Connection, source_id: i64) -> Option<i64> {
    conn.query_row(
        "SELECT id FROM jobs WHERE source_id = ?1 AND kind IN ('sync','reclassify') AND status IN ('pending','running') ORDER BY created_at LIMIT 1",
        [source_id],
        |r| r.get(0),
    )
    .ok()
}

/// Promote pending → running up to MAX_CONCURRENT. Runs inside the caller's
/// `BEGIN IMMEDIATE` (see `maybe_promote` / `create_job`) and returns the
/// promoted ids.
fn promote_pending_inner(conn: &Connection) -> Vec<i64> {
    let running: i64 = conn
        .query_row("SELECT COUNT(*) FROM jobs WHERE status = 'running'", [], |r| r.get(0))
        .unwrap_or(0);
    let slots = (MAX_CONCURRENT as i64) - running;
    if slots <= 0 {
        return vec![];
    }
    let mut stmt = match conn.prepare("SELECT id FROM jobs WHERE status = 'pending' ORDER BY created_at LIMIT ?1") {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    let pending: Vec<i64> = stmt
        .query_map([slots], |r| r.get(0))
        .unwrap()
        .filter_map(|r| r.ok())
        .collect();
    for id in &pending {
        let _ = conn.execute("UPDATE jobs SET status = 'running', started_at = datetime('now') WHERE id = ?1 AND status='pending'", [*id]);
    }
    pending
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    Pending,
    Running,
    Done,
    Failed,
    Cancelled,
}

impl JobStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            JobStatus::Pending => "pending",
            JobStatus::Running => "running",
            JobStatus::Done => "done",
            JobStatus::Failed => "failed",
            JobStatus::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Job {
    pub id: i64,
    pub source_id: i64,
    pub kind: String,
    pub status: JobStatus,
    pub created_at: String,
    pub error: Option<String>,
}

impl Job {
    pub fn from_row(row: &rusqlite::Row) -> rusqlite::Result<Self> {
        Ok(Job {
            id: row.get(0)?,
            source_id: row.get(1)?,
            kind: row.get(2)?,
            status: match row.get::<_, String>(3)?.as_str() {
                "running" => JobStatus::Running,
                "done" => JobStatus::Done,
                "failed" => JobStatus::Failed,
                "cancelled" => JobStatus::Cancelled,
                _ => JobStatus::Pending,
            },
            created_at: row.get(4)?,
            error: row.get(5)?,
        })
    }
}

pub struct JobManager {
    // in-memory queue complements DB persistence (mirrors Python MAX_CONCURRENT window)
    pending: Mutex<VecDeque<i64>>,
    running: Mutex<HashMap<i64, Instant>>,
}

impl JobManager {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            pending: Mutex::new(VecDeque::new()),
            running: Mutex::new(HashMap::new()),
        })
    }

    /// Create a job row and enqueue. Returns job id. Mirrors `backend/app/routers/sources.py: POST /sync`.
    /// R30.1: sync-family kinds (`sync`/`reclassify`) allow at most one
    /// pending/running job per source — the check runs in the same
    /// transaction as the insert, so racing triggers can't both slip
    /// through. A duplicate fails with `Conflict(active_id)`, no new row.
    pub fn create_job(&self, conn: &Connection, source_id: i64, kind: &str) -> Result<i64, CreateJobError> {
        let _guard = PROMOTE_LOCK.lock().unwrap();
        let _ = conn.execute("BEGIN IMMEDIATE", []);
        if is_sync_family(kind) {
            if let Some(active) = active_sync_job(conn, source_id) {
                let _ = conn.execute("ROLLBACK", []);
                return Err(CreateJobError::Conflict(active));
            }
        }
        if let Err(e) = conn.execute(
            "INSERT INTO jobs (source_id, kind, status, total, processed, result, created_at) VALUES (?1, ?2, 'pending', 0, 0, '{}', datetime('now'))",
            rusqlite::params![source_id, kind],
        ) {
            let _ = conn.execute("ROLLBACK", []);
            return Err(CreateJobError::Db(e));
        }
        let id = conn.last_insert_rowid();
        // enforce MAX_CONCURRENT immediately, in the same transaction
        let promoted = promote_pending_inner(conn);
        let _ = conn.execute("COMMIT", []);
        let mut running = self.running.lock().unwrap();
        for pid in promoted {
            running.insert(pid, Instant::now());
        }
        Ok(id)
    }

    /// Try to promote pending -> running up to MAX_CONCURRENT — atomic (P1 4.10).
    /// Uses `BEGIN IMMEDIATE` + global `PROMOTE_LOCK` so concurrent `create_job` cannot double-promote same pending.
    pub fn maybe_promote(&self, conn: &Connection) {
        let _guard = PROMOTE_LOCK.lock().unwrap();
        let _ = conn.execute("BEGIN IMMEDIATE", []);
        let promoted = promote_pending_inner(conn);
        let _ = conn.execute("COMMIT", []);
        let mut running = self.running.lock().unwrap();
        for id in promoted {
            running.insert(id, Instant::now());
        }
    }

    pub fn cancel(&self, conn: &Connection, job_id: i64) -> bool {
        let n = conn
            .execute(
                "UPDATE jobs SET status = 'cancelled' WHERE id = ?1 AND status IN ('pending','running')",
                [job_id],
            )
            .unwrap_or(0);
        if n > 0 {
            self.running.lock().unwrap().remove(&job_id);
            self.maybe_promote(conn);
            return true;
        }
        false
    }

    pub fn complete(&self, conn: &Connection, job_id: i64, success: bool, error: Option<&str>) {
        let status = if success { "done" } else { "failed" };
        let _ = conn.execute(
            "UPDATE jobs SET status = ?1, error = ?2 WHERE id = ?3",
            rusqlite::params![status, error, job_id],
        );
        self.running.lock().unwrap().remove(&job_id);
        self.maybe_promote(conn);
    }

    pub fn list(&self, conn: &Connection, limit: usize) -> Vec<Job> {
        let mut stmt = match conn.prepare("SELECT id, source_id, kind, status, created_at, error FROM jobs ORDER BY created_at DESC LIMIT ?1") {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        stmt.query_map([limit as i64], |r| Job::from_row(r))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect()
    }

    pub fn running(&self, conn: &Connection) -> Vec<Job> {
        let mut stmt = match conn.prepare("SELECT id, source_id, kind, status, created_at, error FROM jobs WHERE status = 'running' ORDER BY created_at") {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        stmt.query_map([], |r| Job::from_row(r)).unwrap().filter_map(|r| r.ok()).collect()
    }
}

impl Default for JobManager {
    fn default() -> Self {
        Self { pending: Mutex::new(VecDeque::new()), running: Mutex::new(HashMap::new()) }
    }
}

// --- helpers for HTTP handlers ---

fn job_json(row: &rusqlite::Row) -> rusqlite::Result<serde_json::Value> {
    let id: i64 = row.get(0)?;
    let source_id: i64 = row.get(1)?;
    let kind: String = row.get(2)?;
    let status: String = row.get(3)?;
    let total: i64 = row.get(4)?;
    let processed: i64 = row.get(5)?;
    let result_str: String = row.get(6)?;
    let error: Option<String> = row.get(7)?;
    let created_at: Option<String> = row.get(8)?;
    let started_at: Option<String> = row.get(9)?;
    let finished_at: Option<String> = row.get(10)?;
    let result: serde_json::Value = serde_json::from_str(&result_str).unwrap_or(serde_json::json!({}));
    Ok(serde_json::json!({
        "id": id,
        "source_id": source_id,
        "kind": kind,
        "status": status,
        "total": total,
        "processed": processed,
        "result": result,
        "error": error,
        "created_at": created_at.unwrap_or_default(),
        "started_at": started_at,
        "finished_at": finished_at
    }))
}

#[derive(Deserialize)]
pub struct JobsQuery {
    pub source_id: Option<i64>,
    pub limit: Option<i64>,
}

pub async fn list_handler(State(state): State<AppState>, Query(q): Query<JobsQuery>) -> Json<serde_json::Value> {
    let data_dir = state.data_dir.clone();
    let result = tokio::task::spawn_blocking(move || {
        let db_path = crate::db::resolve_db_path(&data_dir);
        let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| rusqlite::Connection::open(&db_path).unwrap());
        let mut sql = "SELECT id, source_id, kind, status, total, processed, result, error, created_at, started_at, finished_at FROM jobs".to_string();
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![];
        if let Some(sid) = q.source_id {
            sql.push_str(" WHERE source_id = ?");
            params.push(Box::new(sid));
        }
        sql.push_str(" ORDER BY created_at DESC");
        if let Some(l) = q.limit {
            sql.push_str(&format!(" LIMIT {}", l.clamp(1, 200)));
        } else {
            sql.push_str(" LIMIT 50");
        }
        let mut stmt = match conn.prepare(&sql) {
            Ok(s) => s,
            Err(_) => return serde_json::Value::Array(vec![]),
        };
        let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
        let rows = stmt.query_map(param_refs.as_slice(), |r| job_json(r)).unwrap();
        let out: Vec<serde_json::Value> = rows.filter_map(|r| r.ok()).collect();
        serde_json::Value::Array(out)
    }).await.unwrap();
    Json(result)
}

pub async fn running_handler(State(state): State<AppState>) -> Json<serde_json::Value> {
    let data_dir = state.data_dir.clone();
    let result = tokio::task::spawn_blocking(move || {
        let db_path = crate::db::resolve_db_path(&data_dir);
        let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| rusqlite::Connection::open(&db_path).unwrap());
        let mut stmt = conn.prepare("SELECT id, source_id, kind, status, total, processed, result, error, created_at, started_at, finished_at FROM jobs WHERE status = 'running' ORDER BY created_at").unwrap();
        let rows = stmt.query_map([], |r| job_json(r)).unwrap();
        let out: Vec<serde_json::Value> = rows.filter_map(|r| r.ok()).collect();
        serde_json::Value::Array(out)
    }).await.unwrap();
    Json(result)
}

pub async fn get_handler(State(state): State<AppState>, Path(job_id): Path<i64>) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let data_dir = state.data_dir.clone();
    let result = tokio::task::spawn_blocking(move || {
        let db_path = crate::db::resolve_db_path(&data_dir);
        let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| rusqlite::Connection::open(&db_path).unwrap());
        let mut stmt = conn.prepare("SELECT id, source_id, kind, status, total, processed, result, error, created_at, started_at, finished_at FROM jobs WHERE id = ?1").unwrap();
        match stmt.query_row([job_id], |r| job_json(r)) {
            Ok(v) => Ok(Json(v)),
            Err(_) => Err((StatusCode::NOT_FOUND, Json(serde_json::json!({"detail":"Job not found"})))),
        }
    }).await.unwrap();
    result
}

pub async fn cancel_handler(State(state): State<AppState>, Path(job_id): Path<i64>) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let data_dir = state.data_dir.clone();
    let jobs = state.jobs.clone();
    let result = tokio::task::spawn_blocking(move || {
        let db_path = crate::db::resolve_db_path(&data_dir);
        let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| rusqlite::Connection::open(&db_path).unwrap());
        let exists: Option<String> = conn.query_row("SELECT status FROM jobs WHERE id = ?1", [job_id], |r| r.get(0)).ok();
        let status = match exists {
            Some(s) => s,
            None => return Err((StatusCode::NOT_FOUND, Json(serde_json::json!({"detail":"Job not found"})))),
        };
        if status != "pending" && status != "running" {
            // check if orphaned running (no in-memory task) — treat as cancellable
            // For Rust, we don't track tasks, so we allow cancelling any running
            // If it's done/failed/cancelled already, return 409
            if status == "done" || status == "failed" || status == "cancelled" {
                return Err((StatusCode::CONFLICT, Json(serde_json::json!({"detail": format!("Job is {}, not running", status)}))));
            }
        }
        // If job is running but we have no task, still mark cancelled (orphan handling)
        let was_running = status == "running";
        let cancelled = jobs.cancel(&conn, job_id);
        if cancelled {
            // ensure status is cancelled even if JobManager didn't have it
            let _ = conn.execute("UPDATE jobs SET status='cancelled', finished_at=datetime('now') WHERE id=?1", [job_id]);
            return Ok(Json(serde_json::json!({"cancelled": true})));
        }
        // If cancel returned false but it was pending/running, try direct update
        if status == "pending" || was_running {
            let _ = conn.execute("UPDATE jobs SET status='cancelled', finished_at=datetime('now') WHERE id=?1", [job_id]);
            return Ok(Json(serde_json::json!({"cancelled": true})));
        }
        Err((StatusCode::CONFLICT, Json(serde_json::json!({"detail":"Job is not cancellable"}))))
    }).await.unwrap();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn mk_source(conn: &Connection, name: &str) -> i64 {
        conn.execute("INSERT INTO sources (name, connector, config, enabled, last_sync_cursor, error_count, created_at) VALUES (?1,'folder','{}',1,'',0,datetime('now'))", [name]).unwrap();
        conn.last_insert_rowid()
    }

    #[test]
    fn bounded_concurrency() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.db");
        let conn = crate::db::init_db(&path).unwrap();
        let mgr = JobManager::new();
        // R30.1: one active sync per source, so the global window needs distinct sources
        for i in 0..4 {
            let sid = mk_source(&conn, &format!("s{i}"));
            mgr.create_job(&conn, sid, "sync").unwrap();
        }
        let running: i64 = conn.query_row("SELECT COUNT(*) FROM jobs WHERE status='running'", [], |r| r.get(0)).unwrap();
        let pending: i64 = conn.query_row("SELECT COUNT(*) FROM jobs WHERE status='pending'", [], |r| r.get(0)).unwrap();
        assert_eq!(running, MAX_CONCURRENT as i64);
        assert_eq!(pending, 2);
    }
    #[test]
    fn cancel_pending() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test2.db");
        let conn = crate::db::init_db(&path).unwrap();
        let mgr = JobManager::new();
        // fill running with two sources, third stays pending
        mgr.create_job(&conn, mk_source(&conn, "a"), "sync").unwrap();
        mgr.create_job(&conn, mk_source(&conn, "b"), "sync").unwrap();
        let pend = mgr.create_job(&conn, mk_source(&conn, "c"), "sync").unwrap();
        assert!(mgr.cancel(&conn, pend));
        let status: String = conn.query_row("SELECT status FROM jobs WHERE id=?1", [pend], |r| r.get(0)).unwrap();
        assert_eq!(status, "cancelled");
    }
    #[test]
    fn concurrent_promote_atomic() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test3.db");
        let conn = crate::db::init_db(&path).unwrap();
        // distinct sources per create (R30.1 forbids same-source duplicates)
        let sids: Vec<i64> = (0..4).map(|i| mk_source(&conn, &format!("s{i}"))).collect();
        let mgr = JobManager::new();
        let mgr2 = mgr.clone();
        let path2 = path.clone();
        let path3 = path.clone();
        // concurrent create from 2 threads (each with own connection, same file)
        let (s1, s2) = (sids[0..2].to_vec(), sids[2..4].to_vec());
        let h1 = std::thread::spawn(move || {
            let c = crate::db::open_db(&path2).unwrap();
            for sid in &s1 {
                let _ = mgr2.create_job(&c, *sid, "sync");
            }
        });
        let h2 = std::thread::spawn(move || {
            let c = crate::db::open_db(&path3).unwrap();
            for sid in &s2 {
                let _ = mgr.create_job(&c, *sid, "sync");
            }
        });
        h1.join().unwrap();
        h2.join().unwrap();
        let c = crate::db::open_db(&path).unwrap();
        let running: i64 = c.query_row("SELECT COUNT(*) FROM jobs WHERE status='running'", [], |r| r.get(0)).unwrap();
        let pending: i64 = c.query_row("SELECT COUNT(*) FROM jobs WHERE status='pending'", [], |r| r.get(0)).unwrap();
        // MAX 2 running, rest pending — never exceeds
        assert!(running <= MAX_CONCURRENT as i64);
        assert_eq!(running + pending, 4);
    }

    /// R30.1: sync+sync on one source — the second create conflicts, no new row.
    #[test]
    fn sync_dedup_same_source() {
        let dir = tempdir().unwrap();
        let conn = crate::db::init_db(&dir.path().join("dedup.db")).unwrap();
        let mgr = JobManager::new();
        let sid = mk_source(&conn, "s");
        let first = mgr.create_job(&conn, sid, "sync").unwrap();
        match mgr.create_job(&conn, sid, "sync") {
            Err(CreateJobError::Conflict(active)) => assert_eq!(active, first),
            other => panic!("expected Conflict({}), got {:?}", first, other.map(|_| ())),
        }
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM jobs WHERE source_id=?1", [sid], |r| r.get(0)).unwrap();
        assert_eq!(n, 1, "a duplicate sync must not add a row");
        // other sources are unaffected
        mgr.create_job(&conn, mk_source(&conn, "t"), "sync").unwrap();
    }

    /// R30.1: sync and reclassify share the one slot (either order).
    #[test]
    fn sync_reclassify_share_one_slot() {
        let dir = tempdir().unwrap();
        let conn = crate::db::init_db(&dir.path().join("xkind.db")).unwrap();
        let mgr = JobManager::new();
        let sid = mk_source(&conn, "s");
        let sync = mgr.create_job(&conn, sid, "sync").unwrap();
        assert!(matches!(mgr.create_job(&conn, sid, "reclassify"), Err(CreateJobError::Conflict(id)) if id == sync));
        mgr.cancel(&conn, sync);
        let recl = mgr.create_job(&conn, sid, "reclassify").unwrap();
        assert!(matches!(mgr.create_job(&conn, sid, "sync"), Err(CreateJobError::Conflict(id)) if id == recl));
    }

    /// R30.1: the slot frees when the job finishes or is cancelled.
    #[test]
    fn dedup_clears_on_complete_and_cancel() {
        let dir = tempdir().unwrap();
        let conn = crate::db::init_db(&dir.path().join("free.db")).unwrap();
        let mgr = JobManager::new();
        let sid = mk_source(&conn, "s");
        let a = mgr.create_job(&conn, sid, "sync").unwrap();
        mgr.complete(&conn, a, true, None);
        let b = mgr.create_job(&conn, sid, "sync").unwrap();
        assert_ne!(a, b);
        mgr.cancel(&conn, b);
        let c = mgr.create_job(&conn, sid, "reclassify").unwrap();
        assert_ne!(b, c);
    }

    /// R30.1: `embed` jobs only fill vectors — never guarded, never block sync.
    #[test]
    fn embed_jobs_not_guarded() {
        let dir = tempdir().unwrap();
        let conn = crate::db::init_db(&dir.path().join("embed.db")).unwrap();
        let mgr = JobManager::new();
        let sid = mk_source(&conn, "s");
        mgr.create_job(&conn, sid, "embed").unwrap();
        mgr.create_job(&conn, sid, "embed").unwrap();
        mgr.create_job(&conn, sid, "sync").unwrap();
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM jobs WHERE source_id=?1", [sid], |r| r.get(0)).unwrap();
        assert_eq!(n, 3);
    }

    /// R30.1: racing same-source triggers (watcher + scheduler + manual) —
    /// exactly one row wins, the rest conflict.
    #[test]
    fn concurrent_same_source_single_winner() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("race.db");
        let conn = crate::db::init_db(&path).unwrap();
        let sid = mk_source(&conn, "s");
        drop(conn);
        let mgr = JobManager::new();
        let mgr2 = mgr.clone();
        let (p1, p2) = (path.clone(), path.clone());
        let h1 = std::thread::spawn(move || {
            let c = crate::db::open_db(&p1).unwrap();
            mgr2.create_job(&c, sid, "sync").map(|_| ()).map_err(|e| format!("{e:?}"))
        });
        let h2 = std::thread::spawn(move || {
            let c = crate::db::open_db(&p2).unwrap();
            mgr.create_job(&c, sid, "reclassify").map(|_| ()).map_err(|e| format!("{e:?}"))
        });
        let (r1, r2) = (h1.join().unwrap(), h2.join().unwrap());
        assert!(r1.is_ok() ^ r2.is_ok(), "exactly one racer must win: {r1:?} vs {r2:?}");
        let c = crate::db::open_db(&path).unwrap();
        let n: i64 = c.query_row("SELECT COUNT(*) FROM jobs WHERE source_id=?1", [sid], |r| r.get(0)).unwrap();
        assert_eq!(n, 1);
    }
}
