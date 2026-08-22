//! Scheduler (R4.2) — poll loops `jira_poll_minutes` / `folder_scan_minutes`, `reload_sources` on CRUD.
//! Port of `backend/app/scheduler.py`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::jobs::JobManager;

#[derive(Debug, Clone)]
pub struct SourceSchedule {
    pub source_id: i64,
    pub connector: String,
    pub enabled: bool,
    pub poll_minutes: i64,
    pub last_synced_at: Option<String>,
}

pub struct Scheduler {
    jobs: Arc<JobManager>,
    handles: Mutex<HashMap<i64, tokio::task::JoinHandle<()>>>,
    data_dir: String,
    settings: Arc<crate::settings::SettingsService>,
}

impl Scheduler {
    pub fn new(jobs: Arc<JobManager>, data_dir: String, settings: Arc<crate::settings::SettingsService>) -> Arc<Self> {
        Arc::new(Self { jobs, handles: Mutex::new(HashMap::new()), data_dir, settings })
    }
    // legacy new for tests (uses temp dir + empty settings)
    pub fn new_for_test(jobs: Arc<JobManager>) -> Arc<Self> {
        let dir = std::env::temp_dir().to_string_lossy().to_string();
        let store = crate::secrets::SecretStore::new(std::path::PathBuf::from(&dir).join("secrets.enc"));
        let settings = Arc::new(crate::settings::SettingsService::new(store));
        Arc::new(Self { jobs, handles: Mutex::new(HashMap::new()), data_dir: dir, settings })
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

    /// Spawn a periodic job creator for one source (used by reload). R10.3: actually creates sync jobs.
    pub fn spawn_for_source(&self, source: SourceSchedule) {
        if !source.enabled || source.poll_minutes <= 0 {
            return;
        }
        let jobs = self.jobs.clone();
        let data_dir = self.data_dir.clone();
        let handle = tokio::spawn(async move {
            // For tests, poll_minutes may be small (e.g., 1); use at least 1s for fast tests with 0-minute hack
            let secs = if source.poll_minutes <= 0 { 60 } else { source.poll_minutes as u64 * 60 };
            // Use 1s when poll_minutes is 0 for test harness; but we already filtered <=0
            // For unit test with poll_minutes=1, this is 60s — we still test via manual tick
            let mut interval = tokio::time::interval(Duration::from_secs(secs));
            // Skip first immediate tick — wait for interval
            interval.tick().await;
            loop {
                interval.tick().await;
                tracing::info!("scheduler tick source {} ({})", source.source_id, source.connector);
                let db_path = crate::db::resolve_db_path(&data_dir);
                if let Ok(conn) = crate::db::init_db(&db_path) {
                    let _ = jobs.create_job(&conn, source.source_id, "sync");
                }
            }
        });
        self.handles.lock().unwrap().insert(source.source_id, handle);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::JobManager;
    use std::sync::Arc;
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
    #[test]
    fn intervals_default() {
        let dir = tempdir().unwrap();
        let store = crate::secrets::SecretStore::new(dir.path().join("secrets.enc"));
        let svc = crate::settings::SettingsService::new(store);
        let svc_arc = std::sync::Arc::new(svc);
        let jobs = JobManager::new();
        let sched = Scheduler::new(jobs, dir.path().to_string_lossy().to_string(), svc_arc.clone());
        let (jira, folder) = sched.poll_intervals_minutes(&svc_arc);
        assert_eq!(jira, 15);
        assert_eq!(folder, 60);
    }
}
