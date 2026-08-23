#![allow(dead_code)]
#![allow(clippy::unwrap_used)]
//! AnchorCore Rust binary — R1.2: SQLite + migrations + /health.
//! See `rust/BACKLOG.md:1` and `docs/rust-port.md:1`.
//! Mirrors `backend/app/main.py:130` health shape + `backend/app/db.py:44` PRAGMAs.

mod answer;
mod auth;
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
mod security;
mod settings;
mod sources;
mod stubs;
mod system;
mod watcher;

use axum::{
    middleware,
    routing::{get, post},
    Router,
};
use clap::Parser;
use std::path::PathBuf;
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
    // Double-click app (no env, no --data-dir): use ~/.anchorcore like Python's run_app.py
    // (was `data` — wrong when launched via Finder, CWD is /). Fallback to `data` for dev checkout.
    if let Ok(home) = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")) {
        let p = PathBuf::from(home).join(".anchorcore");
        // use home dir if it exists or can be created, else fallback to ./data for dev
        if p.exists() || std::fs::create_dir_all(&p).is_ok() {
            return p;
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
    // R6.2: ensure log file exists so GET /system/logs finds anchorcore.log (Python creates via RotatingFileHandler)
    let log_path = data_dir.join("anchorcore.log");
    if !log_path.exists() {
        let _ = std::fs::write(&log_path, format!("AnchorCore Rust {} started\n", env!("CARGO_PKG_VERSION")));
    }
    // R6.6: sweep orphan running jobs (like 44b7532) — previous binary crash leaves jobs running and blocks queue
    if let Ok(conn) = db::init_db(&db_path) {
        let n = conn.execute("UPDATE jobs SET status='cancelled', finished_at=datetime('now'), error='orphaned (binary restarted)' WHERE status='running'", []).unwrap_or(0);
        if n > 0 {
            tracing::info!("swept {} orphan running jobs to cancelled", n);
        }
    }
    tracing::info!("Rust anchorcore — data_dir {} db {}", data_dir.display(), db_path.display());

    // R1.4 + R1.5: secret store + settings service (mirrors Python wiring in main.py:55)
    let secret_store = secrets::SecretStore::new(data_dir.join("secrets.enc"));
    let settings_svc = std::sync::Arc::new(settings::SettingsService::new(secret_store));
    let jobs_svc = jobs::JobManager::new();

    let csrf_token = security::generate_csrf_token();
    let embedder = std::sync::Arc::new(embedder::Embedder::new(settings_svc.clone()));
    let state = health::AppState {
        data_dir: data_dir.to_string_lossy().to_string(),
        settings: settings_svc.clone(),
        jobs: jobs_svc.clone(),
        csrf_token: csrf_token.clone(),
        embedder: embedder.clone(),
    };
    tracing::info!("CSRF token generated (per-session)");
    // R11.1: Scheduler — actually wired (was dead code). Boot reload + interval executor.
    let scheduler = scheduler::Scheduler::new_with_state(state.clone());
    scheduler::set_global(scheduler.clone());
    if let Ok(conn) = db::init_db(&db_path) {
        scheduler.reload_sources(&conn);
    }
    tracing::info!("scheduler wired (poll intervals from settings)");
    // Folder watcher: extracted to `watcher::service` (P0 3.3) — gated for `cargo test`
    // Keeps FolderWatcher (mpsc::Receiver !Sync) + HashMap future off test thread stack (8 MB).
    #[cfg(not(test))]
    {
        watcher::service::spawn(state.clone());
    }

    // Ollama: try to launch if installed but not reachable (double-click app should auto-start Ollama)
    // Skip when tests point at localhost:1 or ANCHOR_OLLAMA_BASE_URL is explicitly localhost:1
    {
        let settings_clone = settings_svc.clone();
        tokio::spawn(async move {
            let base = settings_clone.get("ollama_base_url", None).unwrap_or_else(|| "http://localhost:11434".to_string());
            if base.contains("localhost:1") || base.contains("127.0.0.1:1") {
                return;
            }
            // quick probe - if already reachable, nothing to do
            let probe_url = format!("{}/api/tags", base.trim_end_matches('/'));
            let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(2)).build();
            if let Ok(c) = client {
                if let Ok(r) = c.get(&probe_url).send().await {
                    if r.status().is_success() {
                        tracing::info!("ollama already reachable at {}", base);
                        return;
                    }
                }
            }
            // try to find ollama binary
            let candidates = [
                "/opt/homebrew/bin/ollama",
                "/usr/local/bin/ollama",
                "/Applications/Ollama.app/Contents/Resources/ollama",
                "/usr/bin/ollama",
            ];
            let mut ollama_path: Option<std::path::PathBuf> = None;
            for p in candidates {
                if std::path::Path::new(p).exists() {
                    ollama_path = Some(p.into());
                    break;
                }
            }
            if ollama_path.is_none() {
                // try PATH
                if let Ok(out) = std::process::Command::new("which").arg("ollama").output() {
                    if out.status.success() {
                        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
                        if !s.is_empty() && std::path::Path::new(&s).exists() {
                            ollama_path = Some(s.into());
                        }
                    }
                }
            }
            // also try `ollama` via PATH directly
            if ollama_path.is_none() {
                // check if `ollama` is runnable via `command -v` style - try spawning `ollama --version`
                if std::process::Command::new("ollama").arg("--version").output().map(|o| o.status.success()).unwrap_or(false) {
                    ollama_path = Some("ollama".into());
                }
            }
            let Some(path) = ollama_path else {
                tracing::info!("ollama not found in PATH or common locations, skipping auto-launch (install from https://ollama.com)");
                return;
            };
            tracing::info!("ollama not reachable at {}, trying to launch via {:?} serve", base, path);
            let _ = std::process::Command::new(&path)
                .arg("serve")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .stdin(std::process::Stdio::null())
                .spawn();
            // poll for 5s
            for _ in 0..10 {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                if let Ok(c) = reqwest::Client::builder().timeout(std::time::Duration::from_secs(1)).build() {
                    if let Ok(r) = c.get(&probe_url).send().await {
                        if r.status().is_success() {
                            tracing::info!("ollama launched and reachable at {}", base);
                            return;
                        }
                    }
                }
            }
            tracing::info!("ollama launch attempted but still not reachable at {} (will use rule fallback)", base);
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
        // settings (R6.1)
        .route("/settings", get(settings::get_handler).put(settings::put_handler))
        .route("/settings/test-connection", post(settings::test_connection_handler))
        // system (R6.2)
        .route("/system/status", get(system::status_handler))
        .route("/system/onboarding", get(system::onboarding_handler))
        .route("/system/errors", get(system::errors_handler))
        .route("/system/logs", get(system::logs_handler))
        .route("/system/logs/:name", get(system::log_download_handler))
        // auth (R6.3 B40)
        .route("/auth/status", get(auth::status_handler))
        .route("/auth/signup", post(auth::signup_handler))
        .route("/auth/login", post(auth::login_handler))
        .route("/auth/me", get(auth::me_handler))
        // R7.1: CSRF token endpoint (public, used by SPA to fetch per-session token)
        .route("/csrf", get(security::csrf_handler))
        // frontend (R5.1) — must be last, SPA fallback to index.html
        .fallback(frontend::handler)
        // Layers: outermost -> innermost: Host/Origin -> CORS -> CSRF -> Auth
        .layer(middleware::from_fn_with_state(state.clone(), auth::require_auth_middleware))
        .layer(middleware::from_fn_with_state(state.clone(), security::csrf_middleware))
        .layer(security::cors_layer())
        .layer(middleware::from_fn_with_state(state.clone(), security::host_origin_middleware))
        .with_state(state);

    let addr = format!("127.0.0.1:{}", args.port);
    tracing::info!("listening on {}", addr);
    // Double-click app: open browser unless ANCHOR_OPEN_BROWSER=0 (CI/tests set 0)
    let open_browser = std::env::var("ANCHOR_OPEN_BROWSER").as_deref() != Ok("0");
    if open_browser {
        let url = format!("http://{}", addr);
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(800)).await;
            let _ = open::that(&url);
            tracing::info!("opened browser at {}", url);
        });
    }
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn qa_handler(
    axum::extract::State(state): axum::extract::State<health::AppState>,
    axum::Json(req): axum::Json<answer::AskRequest>,
) -> axum::Json<answer::AskResponse> {
    let resp = answer::ask(&state.settings, &state.embedder, req, &state.data_dir).await;
    axum::Json(resp)
}

async fn qa_public_handler(
    axum::extract::State(state): axum::extract::State<health::AppState>,
    axum::Json(mut req): axum::Json<answer::AskRequest>,
) -> axum::Json<answer::AskResponse> {
    req.public_only = Some(true);
    let resp = answer::ask(&state.settings, &state.embedder, req, &state.data_dir).await;
    axum::Json(resp)
}

async fn search_handler(
    axum::extract::State(state): axum::extract::State<health::AppState>,
    axum::Json(req): axum::Json<answer::SearchRequest>,
) -> axum::Json<answer::SearchResponse> {
    let resp = answer::search(&state.settings, &state.embedder, req, &state.data_dir).await;
    axum::Json(resp)
}
