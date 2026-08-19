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
mod projects;
mod retrieval;
mod review;
mod scheduler;
mod secrets;
mod settings;
mod stubs;

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
    let db_path = data_dir.join("anchorcore.db");
    // init DB for side-effect (migrations, pragmas) - health/qa open per-request
    let _ = db::init_db(&db_path).expect("failed to init DB");
    tracing::info!("Rust anchorcore — data_dir {} db {}", data_dir.display(), db_path.display());

    // R1.4 + R1.5: secret store + settings service (mirrors Python wiring in main.py:55)
    let secret_store = secrets::SecretStore::new(data_dir.join("secrets.enc"));
    let settings_svc = std::sync::Arc::new(settings::SettingsService::new(secret_store));

    let state = health::AppState {
        data_dir: data_dir.to_string_lossy().to_string(),
        settings: settings_svc,
    };

    // R1.3: stub all routers with 501, keep /health real (already done in R1.2)
    // Note: axum 0.7 uses `/:id` style; keep literal routes before param routes
    let app = Router::new()
        .route("/health", get(health::health))
        // sources
        .route("/sources", get(stubs::not_implemented).post(stubs::not_implemented))
        .route("/sources/jira/projects", post(stubs::not_implemented))
        .route("/sources/jobs", get(stubs::not_implemented))
        .route("/sources/jobs/running", get(stubs::not_implemented))
        .route("/sources/jobs/:id", get(stubs::not_implemented))
        .route("/sources/jobs/:id/cancel", post(stubs::not_implemented))
        .route("/sources/:id", get(stubs::not_implemented).put(stubs::not_implemented).delete(stubs::not_implemented))
        .route("/sources/:id/config", get(stubs::not_implemented))
        .route("/sources/:id/sync", post(stubs::not_implemented))
        .route("/sources/:id/reclassify", post(stubs::not_implemented))
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
        // qa (R2.2)
        .route("/qa", post(qa_handler))
        .route("/qa/public", post(qa_public_handler))
        // settings
        .route("/settings", get(stubs::not_implemented).put(stubs::not_implemented))
        .route("/settings/test-connection", post(stubs::not_implemented))
        // system
        .route("/system/status", get(stubs::not_implemented))
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
