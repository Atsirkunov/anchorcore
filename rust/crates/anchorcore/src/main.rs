//! AnchorCore Rust binary — R1.2: SQLite + migrations + /health.
//! See `rust/BACKLOG.md:1` and `docs/rust-port.md:1`.
//! Mirrors `backend/app/main.py:130` health shape + `backend/app/db.py:44` PRAGMAs.

mod answer;
mod chunking;
mod classifier;
mod connectors;
mod db;
mod distill;
mod embedder;
mod entities;
mod frontend;
mod hashing;
mod health;
mod jobs;
mod pii;
mod pipeline;
mod projects;
mod retrieval;
mod review;
mod scheduler;
mod secrets;
mod settings;
mod sources;
mod stubs;
mod system;

use axum::{
    routing::{get, patch, post},
    Router,
};
use clap::Parser;
use std::path::PathBuf;
use tower_http::cors::CorsLayer;
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
struct Args {
    #[arg(long, default_value = "8000")]
    port: u16,
    #[arg(long, default_value = "")]
    data_dir: String,
}

fn resolve_data_dir(cli: &str) -> PathBuf {
    if !cli.is_empty() {
        return PathBuf::from(cli);
    }
    if let Ok(v) = std::env::var("ANCHOR_DATA_DIR") {
        if !v.is_empty() {
            return PathBuf::from(v);
        }
    }
    if let Ok(v) = std::env::var("ANCHOR_DATABASE_URL") {
        // sqlite:///path — extract path
        if let Some(p) = v.strip_prefix("sqlite:///") {
            if let Some(parent) = PathBuf::from(p).parent() {
                if !parent.as_os_str().is_empty() {
                    return parent.to_path_buf();
                }
            }
        }
    }
    PathBuf::from("data")
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let args = Args::parse();
    let data_dir = resolve_data_dir(&args.data_dir);
    std::fs::create_dir_all(&data_dir).ok();
    let db_path = db::resolve_db_path(&data_dir.to_string_lossy());
    // init DB for side-effect (migrations, pragmas) - health/qa open per-request
    let _ = db::init_db(&db_path).expect("failed to init DB");
    tracing::info!("Rust anchorcore — data_dir {} db {}", data_dir.display(), db_path.display());

    // R1.4 + R1.5: secret store + settings service (mirrors Python wiring in main.py:55)
    let secret_store = secrets::SecretStore::new(data_dir.join("secrets.enc"));
    let settings_svc = std::sync::Arc::new(settings::SettingsService::new(secret_store));
    let jobs_svc = jobs::JobManager::new();

    let state = health::AppState {
        data_dir: data_dir.to_string_lossy().to_string(),
        settings: settings_svc.clone(),
        jobs: jobs_svc.clone(),
    };
    // Folder watcher: spawn background tasks for existing folder sources (test `test_folder_watcher_picks_up_new_files` expects new file to be ingested within 20s)
    {
        let watch_state = state.clone();
        tokio::spawn(async move {
            use std::collections::{HashMap, HashSet};
            use std::path::PathBuf;
            let mut watchers: HashMap<i64, (crate::connectors::watcher::FolderWatcher, PathBuf)> = HashMap::new();
            let mut known_files: HashMap<i64, HashSet<PathBuf>> = HashMap::new();
            loop {
                // discover folder sources (spawn_blocking to keep Connection off async stack)
                let data_dir_clone = watch_state.data_dir.clone();
                let folder_sources: Vec<(i64, String)> = tokio::task::spawn_blocking(move || {
                    let db_path = crate::db::resolve_db_path(&data_dir_clone);
                    let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| rusqlite::Connection::open(&db_path).unwrap());
                    let mut stmt = match conn.prepare("SELECT id, config FROM sources WHERE connector='folder' AND enabled=1") {
                        Ok(s) => s,
                        Err(_) => return vec![],
                    };
                    let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))).unwrap();
                    let mut out = vec![];
                    for row in rows.flatten() {
                        let (id, cfg_str) = row;
                        let cfg: serde_json::Value = serde_json::from_str(&cfg_str).unwrap_or(serde_json::json!({}));
                        if let Some(p) = cfg.get("path").and_then(|v| v.as_str()) {
                            out.push((id, p.to_string()));
                        }
                    }
                    out
                }).await.unwrap_or_default();
                // ensure watchers
                for (sid, path_str) in &folder_sources {
                    if watchers.contains_key(sid) { continue; }
                    let path = PathBuf::from(path_str);
                    let expanded = if path_str.starts_with('~') { PathBuf::from(path_str.replacen('~', &std::env::var("HOME").unwrap_or_default(), 1)) } else { path.clone() };
                    let cfg = serde_json::json!({"path": expanded.to_string_lossy()});
                    if let Ok(w) = crate::connectors::watcher::FolderWatcher::new(&expanded) {
                        // initial file set
                        let mut set = HashSet::new();
                        if let Ok(fc) = crate::connectors::folder::FolderConnector::new(&cfg) {
                            if let Ok((docs, _)) = fc.fetch() {
                                for d in docs { set.insert(PathBuf::from(d.external_id)); }
                            }
                        }
                        known_files.insert(*sid, set);
                        let display_str = expanded.display().to_string();
                        watchers.insert(*sid, (w, expanded));
                        tracing::info!("watcher started for source {} at {}", sid, display_str);
                    }
                }
                let mut to_sync: Vec<i64> = Vec::new();
                for (sid, (watcher, _)) in watchers.iter() {
                    if !watcher.poll(std::time::Duration::from_millis(300)).is_empty() {
                        to_sync.push(*sid);
                    }
                }
                for sid in to_sync {
                    tracing::info!("watcher detected changes for source {}", sid);
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    let cfg_str = {
                        let db_path = crate::db::resolve_db_path(&watch_state.data_dir);
                        let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| rusqlite::Connection::open(&db_path).unwrap());
                        conn.query_row("SELECT config FROM sources WHERE id=?1", [sid], |r| r.get::<_, String>(0)).unwrap_or_default()
                    };
                    let cfg: serde_json::Value = serde_json::from_str(&cfg_str).unwrap_or(serde_json::json!({}));
                    if let Ok(fc) = crate::connectors::folder::FolderConnector::new(&cfg) {
                        if let Ok((docs, _)) = fc.fetch() {
                            let new_set: HashSet<PathBuf> = docs.iter().map(|d| PathBuf::from(&d.external_id)).collect();
                            let old_set = known_files.get(&sid).cloned().unwrap_or_default();
                            if new_set != old_set {
                                known_files.insert(sid, new_set);
                                let data_dir = watch_state.data_dir.clone();
                                let settings = watch_state.settings.clone();
                                let jobs = watch_state.jobs.clone();
                                tokio::spawn(async move {
                                    let db_path = crate::db::resolve_db_path(&data_dir);
                                    let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| rusqlite::Connection::open(&db_path).unwrap());
                                    let job_id = match jobs.create_job(&conn, sid, "sync") {
                                        Ok(id) => id,
                                        Err(_) => return,
                                    };
                                    let classifier = std::sync::Arc::new(crate::classifier::Classifier::new(settings.clone()));
                                    let embedder = std::sync::Arc::new(crate::embedder::Embedder::new(settings.clone()));
                                    let pipeline = crate::pipeline::Pipeline::new(classifier, embedder, settings, data_dir);
                                    pipeline.sync_source(sid, job_id, false).await;
                                });
                            }
                        }
                    }
                }
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
        });
    }

    // R1.3: stub all routers with 501, keep /health real (already done in R1.2)
    // Note: axum 0.7 uses `/:id` style; keep literal routes before param routes
    let app = Router::new()
        .route("/health", get(health::health))
        // sources (R4.5 + pipeline sync)
        .route("/sources", get(sources::list_handler).post(sources::create_handler))
        .route("/sources/jira/projects", post(stubs::not_implemented))
        .route("/sources/jobs", get(jobs::list_handler))
        .route("/sources/jobs/running", get(jobs::running_handler))
        .route("/sources/jobs/:id", get(jobs::get_handler))
        .route("/sources/jobs/:id/cancel", post(jobs::cancel_handler))
        .route("/sources/:id", get(sources::get_handler).put(sources::update_handler).delete(sources::delete_handler))
        .route("/sources/:id/config", get(sources::config_handler))
        .route("/sources/:id/sync", post(sources::sync_handler))
        .route("/sources/:id/reclassify", post(sources::reclassify_handler))
        // entities + review (R4.4)
        .route("/entities", get(entities::list_handler))
        .route("/entities/:id", get(entities::get_handler).patch(entities::patch_handler))
        .route("/entities/:id/related", get(entities::related_handler))
        .route("/entities/:id/dispute", post(entities::dispute_handler))
        .route("/entities/:id/disputes", get(entities::disputes_handler))
        .route("/entities/:id/context", get(entities::context_handler))
        .route("/review/low-confidence", get(review::low_confidence_handler))
        .route("/review/duplicates", get(review::duplicates_handler))
        .route("/review/merge", post(review::merge_handler))
        // pii (R4.3)
        .route("/pii/config", get(pii::get_config_handler).put(pii::put_config_handler))
        .route("/pii/review", get(pii::review_handler))
        .route("/pii/review/:id", post(pii::decide_handler))
        .route("/pii/scan/:id", post(pii::scan_handler))
        // projects (R4.4)
        .route("/projects", get(projects::list_handler).post(projects::create_handler))
        .route("/projects/default", get(projects::default_handler))
        .route("/projects/:id", get(projects::get_handler).patch(projects::patch_handler).delete(projects::delete_handler))
        // qa (R2.2 + R4.5 search)
        .route("/qa", post(qa_handler))
        .route("/qa/public", post(qa_public_handler))
        .route("/qa/search", post(search_handler))
        // settings
        .route("/settings", get(stubs::not_implemented).put(stubs::not_implemented))
        .route("/settings/test-connection", post(stubs::not_implemented))
        // system
        .route("/system/status", get(system::status_handler))
        .route("/system/onboarding", get(stubs::not_implemented))
        .route("/system/errors", get(stubs::not_implemented))
        .route("/system/logs", get(stubs::not_implemented))
        .route("/system/logs/:name", get(stubs::not_implemented))
        // auth
        .route("/auth/status", get(stubs::not_implemented))
        .route("/auth/signup", post(stubs::not_implemented))
        .route("/auth/login", post(stubs::not_implemented))
        .route("/auth/me", get(stubs::not_implemented))
        // frontend (R5.1) — must be last, SPA fallback to index.html
        .fallback(frontend::handler)
        .layer(CorsLayer::permissive())
        .with_state(state);

    let addr = format!("127.0.0.1:{}", args.port);
    tracing::info!("listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn qa_handler(
    axum::extract::State(state): axum::extract::State<health::AppState>,
    axum::Json(req): axum::Json<answer::AskRequest>,
) -> axum::Json<answer::AskResponse> {
    let resp = answer::ask(&state.settings, req, &state.data_dir).await;
    axum::Json(resp)
}

async fn qa_public_handler(
    axum::extract::State(state): axum::extract::State<health::AppState>,
    axum::Json(mut req): axum::Json<answer::AskRequest>,
) -> axum::Json<answer::AskResponse> {
    req.public_only = Some(true);
    let resp = answer::ask(&state.settings, req, &state.data_dir).await;
    axum::Json(resp)
}

async fn search_handler(
    axum::extract::State(state): axum::extract::State<health::AppState>,
    axum::Json(req): axum::Json<answer::SearchRequest>,
) -> axum::Json<answer::SearchResponse> {
    let resp = answer::search(&state.settings, req, &state.data_dir).await;
    axum::Json(resp)
}
