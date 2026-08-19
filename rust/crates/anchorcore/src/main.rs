//! AnchorCore Rust binary — R1.2: SQLite + migrations + /health.
//! See `rust/BACKLOG.md:1` and `docs/rust-port.md:1`.
//! Mirrors `backend/app/main.py:130` health shape + `backend/app/db.py:44` PRAGMAs.

mod db;
mod health;
mod secrets;
mod settings;
mod stubs;

use axum::{
    routing::{get, patch, post},
    Router,
};
use clap::Parser;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
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
    let conn = db::init_db(&db_path).expect("failed to init DB");
    tracing::info!("Rust anchorcore — data_dir {} db {}", data_dir.display(), db_path.display());

    // R1.4 + R1.5: secret store + settings service (mirrors Python wiring in main.py:55)
    let secret_store = secrets::SecretStore::new(data_dir.join("secrets.enc"));
    let settings_svc = std::sync::Arc::new(settings::SettingsService::new(secret_store));
    // keep a clone for health probing (needs to read ANCHOR_ env + DB)
    let health_settings = settings_svc.clone();

    let state = health::AppState {
        db: Arc::new(Mutex::new(conn)),
        data_dir: data_dir.to_string_lossy().to_string(),
        settings: health_settings,
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
        // entities + review
        .route("/entities/:id", get(stubs::not_implemented).patch(stubs::not_implemented))
        .route("/entities/:id/related", get(stubs::not_implemented))
        .route("/entities/:id/dispute", post(stubs::not_implemented))
        .route("/entities/:id/disputes", get(stubs::not_implemented))
        .route("/entities/:id/context", get(stubs::not_implemented))
        .route("/review/low-confidence", get(stubs::not_implemented))
        .route("/review/duplicates", get(stubs::not_implemented))
        .route("/review/merge", post(stubs::not_implemented))
        // pii
        .route("/pii/config", get(stubs::not_implemented).put(stubs::not_implemented))
        .route("/pii/review", get(stubs::not_implemented))
        .route("/pii/review/:id", post(stubs::not_implemented))
        .route("/pii/scan/:id", post(stubs::not_implemented))
        // projects
        .route("/projects", get(stubs::not_implemented).post(stubs::not_implemented))
        .route("/projects/default", get(stubs::not_implemented))
        .route("/projects/:id", patch(stubs::not_implemented).delete(stubs::not_implemented))
        // qa
        .route("/qa", post(stubs::not_implemented))
        .route("/qa/public", post(stubs::not_implemented))
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
        .layer(CorsLayer::permissive())
        .with_state(state);

    let addr = format!("127.0.0.1:{}", args.port);
    tracing::info!("listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
