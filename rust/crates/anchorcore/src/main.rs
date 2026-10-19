#![allow(dead_code)]
#![allow(clippy::unwrap_used)]
// Windows release builds are windowed (no console popup on double-click).
// Debug builds keep the console. The MCP sidecar intentionally stays console
// (it speaks stdio). Panics are appended to anchorcore.log (see below).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! AnchorCore Rust binary — R1.2: SQLite + migrations + /health.
//! See `rust/BACKLOG.md:1` and `docs/rust-port.md:1`.
//! Mirrors `backend/app/main.py:130` health shape + `backend/app/db.py:44` PRAGMAs.

mod answer;
mod auth;
mod backup;
mod chunking;
mod classifier;
mod cli;
mod common;
mod connectors;
mod db;
mod distill;
mod embedder;
mod entities;
mod extract;
mod fernet;
mod frontend;
mod fsbrowse;
mod hashing;
mod health;
mod items;
mod jobs;
mod license;
mod markdown;
mod mcp;
mod mcp_setup;
mod ollama;
mod pipeline;
mod projects;
mod retrieval;
mod review;
mod scheduler;
mod secrets;
mod sections;
mod security;
mod settings;
mod sources;
mod stubs;
mod system;
mod tags;
mod tray;
mod watcher;

use axum::{
    middleware,
    routing::{delete, get, post},
    Router,
};
use clap::Parser;
use std::path::PathBuf;
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(version)]
struct Args {
    #[arg(long, global = true, default_value = "8000")]
    port: u16,
    #[arg(long, global = true, default_value = "")]
    data_dir: String,
    #[arg(long, global = true, default_value = "")]
    host: String,
    /// Disable the system tray icon (Windows/macOS) — headless/CI/Team
    #[arg(long, global = true)]
    no_tray: bool,
    #[command(subcommand)]
    command: Option<Command>,
}

/// R25.4: headless/Team backup commands — same engine as the UI (backup.rs).
#[derive(clap::Subcommand, Debug)]
enum Command {
    /// Export a portable .anchorcore-backup archive
    Export(cli::ExportArgs),
    /// Import a .anchorcore-backup archive (merge default, or replace)
    Import(cli::ImportArgs),
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

fn resolve_host(cli: &str) -> String {
    if !cli.is_empty() {
        return cli.to_string();
    }
    if let Ok(v) = std::env::var("ANCHOR_HOST") {
        if !v.is_empty() {
            return v;
        }
    }
    // Local-first default: the personal app stays loopback-only. The Team
    // container sets --host 0.0.0.0 (or ANCHOR_HOST) explicitly.
    "127.0.0.1".to_string()
}

/// F-01: Team edition fails closed — a non-loopback bind without
/// `ANCHOR_AUTH_SECRET` exposes every endpoint to the LAN (Host/Origin +
/// the public `/csrf` token authenticate nothing). Pure so tests can cover
/// the matrix without env juggling.
fn team_auth_gate(edition_team: bool, host: &str, auth_enabled: bool) -> Result<(), String> {
    if !edition_team || auth_enabled {
        return Ok(());
    }
    if matches!(host.trim(), "127.0.0.1" | "localhost" | "::1") {
        return Ok(());
    }
    Err(format!(
        "Team edition refuses to bind {host} without ANCHOR_AUTH_SECRET: set it (openssl rand -hex 32) or bind loopback (--host 127.0.0.1)"
    ))
}

fn format_panic_message(
    payload: &(dyn std::any::Any + Send),
    location: Option<&std::panic::Location>,
) -> String {
    // Kept separate from install_panic_hook so unit tests can cover the
    // formatting without touching the process-global panic hook.
    let msg = if let Some(s) = payload.downcast_ref::<&str>() {
        s.to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic payload".to_string()
    };
    let loc = location
        .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
        .unwrap_or_else(|| "?".to_string());
    format!("PANIC at {}: {}", loc, msg)
}

fn install_panic_hook(data_dir: &std::path::Path) {
    // Windowed release builds have no console — a startup panic would
    // otherwise vanish. Append panics to anchorcore.log instead.
    let log_path = data_dir.join("anchorcore.log");
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        default_hook(info);
        let line = format_panic_message(info.payload(), info.location());
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&log_path) {
            use std::io::Write;
            let _ = f.write_all(format!("{}\n", line).as_bytes());
        }
    }));
}

#[cfg(test)]
mod logging_tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn rotates_backups_and_drops_oldest() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("anchorcore.log");
        std::fs::write(&path, vec![b'a'; 100]).unwrap();
        rotate_log(&path, 50, 3);
        assert!(dir.path().join("anchorcore.log.1").exists());
        assert!(!path.exists());
        std::fs::write(&path, vec![b'b'; 100]).unwrap();
        rotate_log(&path, 50, 3);
        assert!(dir.path().join("anchorcore.log.2").exists());
        std::fs::write(&path, vec![b'c'; 10]).unwrap();
        rotate_log(&path, 50, 3);
        assert!(path.exists(), "under the cap must not rotate");
    }

    #[test]
    fn writer_appends_lines() {
        use std::io::Write;
        let dir = tempdir().unwrap();
        let path = dir.path().join("anchorcore.log");
        let mut w = RotatingLogWriter::new(path.clone());
        w.write_all(b"hello\n").unwrap();
        w.write_all(b"world\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello\nworld\n");
    }
}

#[cfg(test)]
mod panic_hook_tests {
    use super::format_panic_message;

    #[test]
    fn formats_str_payload_with_location() {
        let payload: &str = "boom";
        let line = format_panic_message(&payload, Some(std::panic::Location::caller()));
        assert!(line.contains("boom"));
        assert!(line.contains("main.rs"));
        assert!(line.starts_with("PANIC at "));
    }

    #[test]
    fn formats_string_payload_without_location() {
        let payload = "owned boom".to_string();
        let line = format_panic_message(&payload, None);
        assert!(line.contains("owned boom"));
        assert!(line.contains("PANIC at ?: "));
    }

    #[test]
    fn unknown_payload_type() {
        let payload = 42i32;
        let line = format_panic_message(&payload, None);
        assert!(line.contains("unknown panic payload"));
    }
}

#[cfg(test)]
mod shutdown_tests {
    /// R34.1: tray Quit fires this Notify — the server must stop, not hang.
    #[tokio::test]
    async fn shutdown_signal_returns_on_notify() {
        let notify = std::sync::Arc::new(tokio::sync::Notify::new());
        let waiter = tokio::spawn(super::shutdown_signal(notify.clone()));
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        notify.notify_one();
        tokio::time::timeout(std::time::Duration::from_secs(5), waiter)
            .await
            .expect("shutdown_signal must return after notify")
            .unwrap();
    }
}

#[cfg(test)]
mod cli_tests {
    use super::Args;
    use clap::CommandFactory;

    /// R29.1: the setup skill checks an on-disk binary with `--version` before
    /// replacing it — keep the flag wired to the package version.
    #[test]
    fn version_flag_matches_package_version() {
        assert_eq!(Args::command().get_version(), Some(env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn team_auth_gate_matrix() {
        // F-01: Team + LAN bind + no secret refuses to boot; secret or
        // loopback boots; personal edition is unaffected.
        assert!(super::team_auth_gate(true, "0.0.0.0", false).is_err());
        assert!(super::team_auth_gate(true, "0.0.0.0", true).is_ok());
        assert!(super::team_auth_gate(true, "127.0.0.1", false).is_ok());
        assert!(super::team_auth_gate(true, "localhost", false).is_ok());
        assert!(super::team_auth_gate(true, "::1", false).is_ok());
        assert!(super::team_auth_gate(false, "0.0.0.0", false).is_ok());
    }

    /// The boot banner must reflect the running binary: write-if-absent
    /// left a 1.0.21 banner at the head of the log after upgrading to 1.0.22.
    #[test]
    fn log_banner_appends_fresh_version_each_boot() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("anchorcore.log");
        std::fs::write(&log, "AnchorCore Rust 0.0.0 started\n").unwrap();
        super::ensure_log_file(dir.path());
        super::ensure_log_file(dir.path());
        let body = std::fs::read_to_string(&log).unwrap();
        let lines: Vec<&str> = body.lines().collect();
        assert_eq!(lines.len(), 3, "each boot appends one banner");
        assert!(lines[2].contains(env!("CARGO_PKG_VERSION")), "banner: {}", lines[2]);
    }
}

fn ensure_log_file(data_dir: &std::path::Path) {
    // R6.2: ensure log file exists so GET /system/logs finds anchorcore.log (Python creates via RotatingFileHandler).
    // Always append a fresh banner: write-if-absent kept the very first
    // boot's banner (e.g. 1.0.21) at the head forever, disagreeing with /system/status.
    let log_path = data_dir.join("anchorcore.log");
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&log_path) {
        use std::io::Write as _;
        let _ = writeln!(f, "AnchorCore Rust {} started", env!("CARGO_PKG_VERSION"));
    }
}

/// R28.3: size-rotating log file — `anchorcore.log` (+ `.1`..`.3` backups),
/// 10 MB per file, mirroring the Python `RotatingFileHandler`.
const LOG_MAX_BYTES: u64 = 10_000_000;
const LOG_BACKUPS: usize = 3;

struct RotatingLogWriter {
    path: PathBuf,
}

impl RotatingLogWriter {
    fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl std::io::Write for RotatingLogWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&self.path)?;
        // rotate before writing the line that would cross the cap
        if f.metadata().map(|m| m.len() >= LOG_MAX_BYTES).unwrap_or(false) {
            drop(f);
            rotate_log(&self.path, LOG_MAX_BYTES, LOG_BACKUPS);
            f = std::fs::OpenOptions::new().create(true).append(true).open(&self.path)?;
        }
        std::io::Write::write_all(&mut f, buf)?;
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn rotate_log(path: &std::path::Path, max_bytes: u64, backups: usize) {
    if std::fs::metadata(path).map(|m| m.len() < max_bytes).unwrap_or(true) {
        return;
    }
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else { return };
    // .2 -> .3, .1 -> .2, base -> .1 (the oldest backup is dropped)
    for i in (1..backups).rev() {
        let from = path.with_file_name(format!("{}.{}", name, i));
        let to = path.with_file_name(format!("{}.{}", name, i + 1));
        if from.exists() {
            let _ = std::fs::rename(&from, &to);
        }
    }
    let _ = std::fs::rename(path, path.with_file_name(format!("{}.1", name)));
}

/// R28.3: stdout + `{data_dir}/anchorcore.log`, INFO by default (Python
/// parity; RUST_LOG still wins). Release builds are windowed so stdout is
/// invisible — without the file layer the app logged nothing but panics.
fn init_logging(data_dir: &std::path::Path) {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let stdout_layer = tracing_subscriber::fmt::layer().with_writer(std::io::stdout);
    let log_path = data_dir.join("anchorcore.log");
    let file_layer = tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .with_writer(move || RotatingLogWriter::new(log_path.clone()));
    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(stdout_layer)
        .with(file_layer)
        .try_init();
}

fn sweep_orphan_jobs(db_path: &std::path::Path) {
    // R6.6: sweep orphan running jobs (like 44b7532) — previous binary crash leaves jobs running and blocks queue
    if let Ok(conn) = db::init_db(db_path) {
        let n = conn.execute("UPDATE jobs SET status='cancelled', finished_at=datetime('now'), error='orphaned (binary restarted)' WHERE status='running'", []).unwrap_or(0);
        if n > 0 {
            tracing::info!("swept {} orphan running jobs to cancelled", n);
        }
    }
}

fn build_state(data_dir: &std::path::Path) -> health::AppState {
    // R1.4 + R1.5: secret store + settings service (mirrors Python wiring in main.py:55)
    let secret_store = secrets::SecretStore::new(data_dir.join("secrets.enc"));
    let settings_svc = std::sync::Arc::new(settings::SettingsService::new(secret_store));
    settings_svc.set_data_dir(data_dir);
    let jobs_svc = jobs::JobManager::new();
    let csrf_token = security::generate_csrf_token();
    let embedder = std::sync::Arc::new(embedder::Embedder::new(settings_svc.clone()));
    health::AppState {
        data_dir: data_dir.to_string_lossy().to_string(),
        settings: settings_svc.clone(),
        jobs: jobs_svc.clone(),
        csrf_token: csrf_token.clone(),
        embedder: embedder.clone(),
    }
}

fn wire_scheduler(state: &health::AppState, db_path: &std::path::Path) {
    // R11.1: Scheduler — actually wired (was dead code). Boot reload + interval executor.
    let scheduler = scheduler::Scheduler::new_with_state(state.clone());
    scheduler::set_global(scheduler.clone());
    if let Ok(conn) = db::init_db(db_path) {
        scheduler.reload_sources(&conn);
    }
    scheduler.spawn_backfill();
    tracing::info!("scheduler wired (poll intervals from settings)");
}

fn spawn_ollama_autolaunch(settings_svc: std::sync::Arc<settings::SettingsService>) {
    // Ollama: try to launch if installed but not reachable (double-click app should auto-start Ollama)
    // Skip when tests point at localhost:1 or ANCHOR_OLLAMA_BASE_URL is explicitly localhost:1
    tokio::spawn(async move {
        let base = settings_svc.get("ollama_base_url", None).unwrap_or_else(|| "http://localhost:11434".to_string());
        if system::ollama_probe_disabled(&base) {
            return;
        }
        // quick probe - if already reachable, nothing to do
        if system::probe_ollama(&base, 2).await {
            tracing::info!("ollama already reachable at {}", base);
            return;
        }
        // try to find ollama binary
        let Some(path) = system::find_ollama_binary() else {
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
        let probe_url = format!("{}/api/tags", base.trim_end_matches('/'));
        if system::poll_ollama_up(&probe_url, 10).await {
            tracing::info!("ollama launched and reachable at {}", base);
            return;
        }
        tracing::info!("ollama launch attempted but still not reachable at {} (will use rule fallback)", base);
    });
}

fn build_router(state: health::AppState) -> Router {
    // R1.3: stub all routers with 501, keep /health real (already done in R1.2)
    // Note: axum 0.7 uses `/:id` style; keep literal routes before param routes
    Router::new()
        .route("/health", get(health::health))
        // sources (R4.5 + pipeline sync)
        .route("/sources", get(sources::list_handler).post(sources::create_handler))
        .route("/sources/jira/projects", post(stubs::not_implemented))
        .route("/sources/rest/preview", post(sources::rest_preview_handler))
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
        .route("/entities/:id/conflicts", get(entities::conflicts_list_handler).post(entities::conflict_create_handler))
        .route("/entities/:id/conflicts/:other_id", delete(entities::conflict_delete_handler))
        .route("/items", get(items::list_handler))
        .route("/items/:id", get(items::get_handler).patch(items::patch_handler))
        .route("/items/:id/outline", get(sections::outline_handler))
        .route("/entities/:id/context", get(entities::context_handler))
        .route("/review/low-confidence", get(review::low_confidence_handler))
        .route("/review/duplicates", get(review::duplicates_handler))
        .route("/review/merge", post(review::merge_handler))
        // projects (R4.4)
        .route("/projects", get(projects::list_handler).post(projects::create_handler))
        .route("/projects/default", get(projects::default_handler))
        .route("/projects/:id", get(projects::get_handler).patch(projects::patch_handler).delete(projects::delete_handler))
        // sections (R14.1)
        .route("/sections", get(sections::list_handler))
        .route("/sections/:id/chunks", get(sections::chunks_handler))
        // nodes (R36.6) — single tree-node lookup with opt-in links
        .route("/nodes/:id", get(sections::node_handler))
        // tags (R14.4)
        .route("/tags", get(tags::list_handler))
        .route("/tags/:id/chunks", get(tags::chunks_handler))
        // qa (R2.2 + R4.5 search)
        .route("/qa", post(qa_handler))
        .route("/qa/public", post(qa_public_handler))
        .route("/qa/search", post(search_handler))
        // settings (R6.1)
        .route("/settings", get(settings::get_handler).put(settings::put_handler))
        .route("/settings/test-connection", post(settings::test_connection_handler))
        // system (R6.2 + R11 manual ollama start)
        .route("/system/status", get(system::status_handler))
        .route("/system/onboarding", get(system::onboarding_handler))
        .route("/system/update", get(system::update_handler))
        .route("/system/errors", get(system::errors_handler))
        .route("/system/fs", get(fsbrowse::fs_browse_handler))
        .route("/system/mcp", get(system::mcp_info_handler))
        .route("/system/mcp/clients", get(mcp_setup::clients_handler))
        .route("/system/mcp/connect", post(mcp_setup::connect_handler))
        .route("/system/mcp/disconnect", post(mcp_setup::disconnect_handler))
        .route("/system/logs", get(system::logs_handler))
        .route("/system/logs/:name", get(system::log_download_handler))
        .route("/system/ollama/start", post(system::ollama_start_handler))
        .route("/system/ollama/pull", post(ollama::pull_handler))
        .route("/system/ollama/pulls", get(ollama::pulls_handler))
        .route("/system/skills/:name", get(system::skills_handler))
        // auth (R6.3 B40)
        .route("/auth/status", get(auth::status_handler))
        .route("/auth/signup", post(auth::signup_handler))
        .route("/auth/login", post(auth::login_handler))
        .route("/auth/me", get(auth::me_handler))
        // R7.1: CSRF token endpoint (public, used by SPA to fetch per-session token)
        .route("/csrf", get(security::csrf_handler))
        // backup export (R25.2)
        .route("/backup/export", post(backup::export_handler))
        // backup import (R25.3)
        .route("/backup/import", post(backup::import_handler))
        // frontend (R5.1) — must be last, SPA fallback to index.html
        .fallback(frontend::handler)
        // Layers: outermost -> innermost: Host/Origin -> CORS -> CSRF -> Auth
        .layer(middleware::from_fn_with_state(state.clone(), auth::require_auth_middleware))
        .layer(middleware::from_fn_with_state(state.clone(), security::csrf_middleware))
        .layer(security::cors_layer())
        .layer(middleware::from_fn_with_state(state.clone(), security::host_origin_middleware))
        .with_state(state)
}

fn maybe_open_browser(addr: &str) {
    // Double-click app: open browser unless ANCHOR_OPEN_BROWSER=0 (CI/tests set 0)
    let open_browser = std::env::var("ANCHOR_OPEN_BROWSER").as_deref() != Ok("0");
    if !open_browser {
        return;
    }
    let url = format!("http://{}", addr);
    let url_clone = url.clone();
    let addr_clone = addr.to_string();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(800)).await;
        let res = crate::tray::open_url(&url_clone);
        match res {
            Ok(_) => tracing::info!("opened browser at {}", url_clone),
            Err(e) => tracing::warn!("failed to open browser at {}: {} (try open http://{} manually)", url_clone, e, addr_clone),
        }
    });
}

/// R34.1: graceful shutdown — tray Quit, Ctrl+C, or SIGTERM (unix), whichever first.
async fn shutdown_signal(notify: std::sync::Arc<tokio::sync::Notify>) {
    wait_for_shutdown(notify).await;
    tracing::info!("shutdown requested");
}

async fn wait_for_quit_or_ctrl_c(notify: &tokio::sync::Notify) {
    tokio::select! {
        _ = notify.notified() => {},
        _ = tokio::signal::ctrl_c() => {},
    }
}

#[cfg(not(unix))]
async fn wait_for_shutdown(notify: std::sync::Arc<tokio::sync::Notify>) {
    wait_for_quit_or_ctrl_c(&notify).await;
}

#[cfg(unix)]
async fn wait_for_shutdown(notify: std::sync::Arc<tokio::sync::Notify>) {
    match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
        Ok(mut sig) => {
            tokio::select! {
                _ = notify.notified() => {},
                _ = tokio::signal::ctrl_c() => {},
                _ = sig.recv() => {},
            }
        }
        Err(e) => {
            tracing::warn!(
                "SIGTERM handler unavailable ({}), Ctrl+C + tray Quit only",
                e
            );
            wait_for_quit_or_ctrl_c(&notify).await;
        }
    }
}

fn mcp_mode_requested() -> bool {
    // R20.8: `anchorcore mcp` runs the stdio MCP server. A copy/rename to
    // `anchorcore-mcp[.exe]` keeps pre-merge client configs working.
    let mut args = std::env::args();
    let argv0 = args.next().unwrap_or_default();
    if let Some(first) = args.next() {
        if first == "mcp" {
            return true;
        }
    }
    let stem = std::path::Path::new(&argv0)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    stem == "anchorcore-mcp"
}

#[tokio::main]
async fn main() {
    // R20.8: stdio MCP mode — no server, no DB init, no browser.
    if mcp_mode_requested() {
        mcp::run().await;
        return;
    }
    // Parse first so `--help`/`--version` work (clap exits before logging;
    // the Team image license gate below logs to the file too).
    let args = Args::parse();
    let data_dir = resolve_data_dir(&args.data_dir);
    std::fs::create_dir_all(&data_dir).ok();
    init_logging(&data_dir);

    // Team edition (Docker image sets ANCHOR_EDITION=team): refuse to boot
    // without a valid license; lapsed maintenance only warns (perpetual).
    if std::env::var("ANCHOR_EDITION").as_deref() == Ok(license::TEAM_EDITION) {
        if let Err(msg) = license::enforce_team_license() {
            tracing::error!("{}", msg);
            eprintln!("{msg}");
            std::process::exit(2);
        }
    }
    // R25.4: CLI subcommands run the backup engine and exit — no server, no
    // watcher, no scheduler.
    if let Some(command) = args.command {
        let code = match command {
            Command::Export(export) => cli::run_export(&data_dir, args.port, export).await,
            Command::Import(import) => cli::run_import(&data_dir, args.port, import).await,
        };
        std::process::exit(code);
    }
    // F-01: Team fails closed on LAN binds without auth (mirrors the license
    // gate above; CLI export/import already exited — they serve nothing).
    if std::env::var("ANCHOR_EDITION").as_deref() == Ok(license::TEAM_EDITION) {
        if let Err(msg) = team_auth_gate(true, &resolve_host(&args.host), auth::auth_enabled()) {
            tracing::error!("{}", msg);
            eprintln!("{msg}");
            std::process::exit(2);
        }
    }
    install_panic_hook(&data_dir);
    let db_path = db::resolve_db_path(&data_dir.to_string_lossy());
    // init DB for side-effect (migrations, pragmas) - health/qa open per-request
    let _ = db::init_db(&db_path).expect("failed to init DB");
    ensure_log_file(&data_dir);
    sweep_orphan_jobs(&db_path);
    tracing::info!("Rust anchorcore — data_dir {} db {}", data_dir.display(), db_path.display());

    let state = build_state(&data_dir);
    tracing::info!("CSRF token generated (per-session)");
    // R29.1: opt-in startup update check (`update_check_enabled`); no-op when off.
    system::spawn_update_check(state.clone());
    wire_scheduler(&state, &db_path);
    // Folder watcher: extracted to `watcher::service` (P0 3.3) — gated for `cargo test`
    // Keeps FolderWatcher (mpsc::Receiver !Sync) + HashMap future off test thread stack (8 MB).
    #[cfg(not(test))]
    {
        watcher::service::spawn(state.clone());
    }

    spawn_ollama_autolaunch(state.settings.clone());

    let app = build_router(state);

    let addr = format!("{}:{}", resolve_host(&args.host), args.port);
    tracing::info!("listening on {}", addr);
    maybe_open_browser(&addr);
    // Bind via std first: a second instance fails here before it can plant a
    // tray icon, and tray creation below stays on the main thread (macOS).
    let std_listener = std::net::TcpListener::bind(&addr).unwrap();
    std_listener.set_nonblocking(true).unwrap();
    let shutdown = std::sync::Arc::new(tokio::sync::Notify::new());
    let _tray = tray::spawn_if_enabled(&addr, args.no_tray, shutdown.clone());
    let listener = tokio::net::TcpListener::from_std(std_listener).unwrap();
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal(shutdown))
        .await
        .unwrap();
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
