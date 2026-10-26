//! System status — port of `backend/app/routers/system.py:64` `GET /system/status`.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

use crate::health::AppState;

fn provider_is_local(base: &str) -> bool {
    crate::common::provider_is_local(base)
}

pub fn ollama_probe_disabled(base: &str) -> bool {
    base == "http://localhost:1" || base == "http://127.0.0.1:1" || base == "http://localhost:1/" || base == "http://127.0.0.1:1/"
}

pub async fn probe_ollama(base: &str, timeout_secs: u64) -> bool {
    // real probe like health.rs (was heuristic localhost:1 and always false)
    let url = format!("{}/api/tags", base.trim_end_matches('/'));
    match reqwest::Client::builder().timeout(std::time::Duration::from_secs(timeout_secs)).build() {
        Ok(c) => c.get(&url).send().await.map(|r| r.status().is_success()).unwrap_or(false),
        Err(_) => false,
    }
}

struct StatusSnapshot {
    ollama_base: String,
    embed_model: String,
    embed_base: String,
    answer_model: String,
    answer_base: String,
    answer_api_key: String,
    answer_provider: String,
}

fn status_snapshot(settings: &crate::settings::SettingsService) -> StatusSnapshot {
    let ollama_base = settings.get("ollama_base_url", None).unwrap_or_else(|| "http://localhost:11434".to_string());
    let embed_model = settings.get("embed_model", None).unwrap_or_else(|| "nomic-embed-text".to_string());
    let embed_base = settings
        .get("embed_base_url", None)
        .or_else(|| settings.get("ollama_base_url", None))
        .unwrap_or_else(|| "http://localhost:11434".to_string());
    let answer_model = settings.get("answer_model", None).unwrap_or_else(|| "gpt-4o-mini".to_string());
    let answer_base = settings.get("answer_base_url", None).unwrap_or_else(|| "https://api.openai.com/v1".to_string());
    let answer_api_key = settings.get("answer_api_key", None).unwrap_or_default();
    let answer_provider = if provider_is_local(&answer_base) {
        "ollama".to_string()
    } else if !answer_api_key.trim().is_empty() {
        "configured".to_string()
    } else {
        "missing".to_string()
    };
    StatusSnapshot {
        ollama_base,
        embed_model,
        embed_base,
        answer_model,
        answer_base,
        answer_api_key,
        answer_provider,
    }
}

/// R45.18: entity extraction is model-free — `rules` or `off`.
fn extraction_mode(settings: &crate::settings::SettingsService) -> &'static str {
    match crate::classifier::extract_mode(settings) {
        crate::classifier::ExtractMode::Off => "off",
        crate::classifier::ExtractMode::Rules => "rules",
    }
}

pub async fn status_handler(State(state): State<AppState>) -> Json<Value> {
    let data_dir = state.data_dir.clone();
    // capture settings values before blocking (use cached snapshot without DB conn for speed)
    let snap = status_snapshot(&state.settings);
    let readiness = crate::ollama::model_readiness(&state.settings).await;
    let llm_mode = crate::settings::llm_mode(&state.settings);
    let answer_ready = crate::answer::answer_ready(&state.settings, readiness.answer_ready);
    let embed_is_local = provider_is_local(&snap.embed_base);
    let extraction = extraction_mode(&state.settings);
    let result = tokio::task::spawn_blocking(move || {
        let db_path = crate::db::resolve_db_path(&data_dir);
        let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| rusqlite::Connection::open(&db_path).unwrap());
        let pending: i64 = conn.query_row("SELECT COUNT(*) FROM chunks WHERE embedding IS NULL", [], |r| r.get(0)).unwrap_or(0);
        let (active_fp, stale_vectors) = crate::embed_fp::stale_vector_status(&conn);
        let mut stmt = conn.prepare("SELECT id, name, last_error, error_count FROM sources WHERE error_count > 0 AND enabled = 1").unwrap();
        let failing: Vec<Value> = stmt.query_map([], |r| Ok(serde_json::json!({
            "id": r.get::<_, i64>(0)?,
            "name": r.get::<_, String>(1)?,
            "error": r.get::<_, Option<String>>(2)?,
            "count": r.get::<_, i64>(3)?
        }))).unwrap().filter_map(|r| r.ok()).collect();
        let version = env!("CARGO_PKG_VERSION");
        serde_json::json!({
            "version": version,
            "data_dir": "redacted",
            "database": "redacted",
            "ollama": {
                "reachable": readiness.reachable,
                "base_url": snap.ollama_base,
                "embed_model": snap.embed_model,
                "missing_models": readiness.missing_models
            },
            "extraction": { "mode": extraction },
            "retrieval": {
                "calls": 0,
                "vec0_calls": 0,
                "avg_latency_ms": 0.0,
                "backend": "vec0"
            },
            "embedder": {
                "provider": if embed_is_local { "local" } else { "cloud" },
                "base_url": snap.embed_base,
                "model": snap.embed_model,
                // R45.5: fingerprint of the latest query vector + stored
                // vectors the vector legs skip until the backfill re-embeds.
                "active_fingerprint": active_fp,
                "stale_vectors": stale_vectors,
            },
            // R45.2: who writes answers — "agent" (the user's AI agent via
            // MCP or the REST skill; no server chat model) or "local".
            "llm_mode": llm_mode,
            "answer": {
                "provider": snap.answer_provider,
                "model": snap.answer_model,
                "base_url": snap.answer_base,
                "mode": llm_mode,
            },
            "answer_ready": answer_ready,
            "pending_embeddings": pending,
            "failing_sources": failing,
            "tasks": {}
        })
    }).await.unwrap();
    Json(result)
}

#[derive(Deserialize)]
pub struct ErrorsQuery {
    pub limit: Option<i64>,
    pub component: Option<String>,
    pub level: Option<String>,
}

fn error_row(conn: &rusqlite::Connection, r: &rusqlite::Row) -> rusqlite::Result<Value> {
    let id: i64 = r.get(0)?;
    let component: String = r.get(1)?;
    let level: String = r.get(2)?;
    let source_id: Option<i64> = r.get(3)?;
    let message: String = r.get(4)?;
    let detail: String = r.get(5)?;
    let created_at: String = r.get(6)?;
    let source_name: Option<String> = match source_id {
        Some(sid) => conn.query_row("SELECT name FROM sources WHERE id=?1", [sid], |rr| rr.get(0)).ok(),
        None => None,
    };
    Ok(serde_json::json!({
        "id": id, "component": component, "level": level,
        "source_id": source_id, "source_name": source_name,
        "message": message, "detail": detail, "created_at": created_at
    }))
}

pub async fn errors_handler(
    State(state): State<AppState>,
    Query(q): Query<ErrorsQuery>,
) -> Json<Value> {
    let data_dir = state.data_dir.clone();
    let result = tokio::task::spawn_blocking(move || {
        let db_path = crate::db::resolve_db_path(&data_dir);
        let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| rusqlite::Connection::open(&db_path).unwrap());
        let limit = q.limit.unwrap_or(50).clamp(1, 200);
        let component = q.component;
        let level = q.level;
        let mut sql = "SELECT id, component, level, source_id, message, detail, created_at FROM system_events".to_string();
        let mut clauses = vec![];
        if component.is_some() {
            clauses.push("component = ?");
        }
        if level.is_some() {
            clauses.push("level = ?");
        }
        if !clauses.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&clauses.join(" AND "));
        }
        sql.push_str(" ORDER BY created_at DESC LIMIT ?");
        let mut stmt = conn.prepare(&sql).unwrap();
        let mut params: Vec<&dyn rusqlite::types::ToSql> = vec![];
        if let Some(c) = &component {
            params.push(c);
        }
        if let Some(l) = &level {
            params.push(l);
        }
        params.push(&limit);
        let map = stmt.query_map(rusqlite::params_from_iter(params), |r| error_row(&conn, r)).unwrap();
        let rows: Vec<Value> = map.filter_map(|r| r.ok()).collect();
        Value::Array(rows)
    })
    .await
    .unwrap();
    Json(result)
}

/// R20.2 + R20.8 — canonical way to run this app's stdio MCP mode
/// (`<this exe> mcp`, one executable) + audit counters, so the UI shows a
/// copy-paste client command instead of a `cargo run` line.
pub async fn mcp_info_handler(State(state): State<AppState>) -> Json<Value> {
    let command = std::env::current_exe().ok().map(|p| p.to_string_lossy().to_string());
    let data_dir = state.data_dir.clone();
    // R20.11: a non-default data dir rides along so `anchorcore mcp` finds runtime.json.
    let args = crate::runtime::mcp_args(std::path::Path::new(&data_dir));
    let result = tokio::task::spawn_blocking(move || {
        let db_path = crate::db::resolve_db_path(&data_dir);
        let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| rusqlite::Connection::open(&db_path).unwrap());
        let calls: i64 = conn
            .query_row("SELECT COUNT(*) FROM system_events WHERE component = 'mcp'", [], |r| r.get(0))
            .unwrap_or(0);
        let last_call_at: Option<String> = conn
            .query_row("SELECT created_at FROM system_events WHERE component = 'mcp' ORDER BY created_at DESC LIMIT 1", [], |r| r.get(0))
            .ok();
        serde_json::json!({
            "mode": "bundled",
            "command": command,
            "args": args,
            "calls": calls,
            "last_call_at": last_call_at,
        })
    })
    .await
    .unwrap();
    Json(result)
}

pub async fn onboarding_handler(State(state): State<AppState>) -> Json<Value> {
    let data_dir = state.data_dir.clone();
    let settings = state.settings.clone();
    let readiness = crate::ollama::model_readiness(&settings).await;
    let answer_ready = crate::answer::answer_ready(&settings, readiness.answer_ready);
    let result = tokio::task::spawn_blocking(move || {
        let db_path = crate::db::resolve_db_path(&data_dir);
        let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| rusqlite::Connection::open(&db_path).unwrap());
        let sources_count: i64 = conn.query_row("SELECT COUNT(*) FROM sources", [], |r| r.get(0)).unwrap_or(0);
        let needs_wizard = sources_count == 0;
        let ollama_base = settings.get("ollama_base_url", None).unwrap_or_else(|| "http://localhost:11434".to_string());
        let answer_base = settings.get("answer_base_url", None).unwrap_or_else(|| "https://api.openai.com/v1".to_string());
        let answer_api_key = settings.get("answer_api_key", None).unwrap_or_default();
        let answer_provider = if crate::common::provider_is_local(&answer_base) {
            "ollama"
        } else if !answer_api_key.trim().is_empty() {
            "configured"
        } else {
            "missing"
        };
        // sample dir resolution like Python _resolve_sample_dir
        let sample_available = {
            let p1 = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../sample");
            let p2 = PathBuf::from("sample");
            let p3 = PathBuf::from(&data_dir).join("../sample");
            p1.exists() || p2.exists() || p3.exists()
        };
        let sample_path = if sample_available {
            let p1 = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../sample");
            if p1.exists() {
                Some(p1.to_string_lossy().to_string())
            } else if PathBuf::from("sample").exists() {
                Some(PathBuf::from("sample").to_string_lossy().to_string())
            } else {
                Some(PathBuf::from(&data_dir).join("../sample").to_string_lossy().to_string())
            }
        } else {
            None
        };
        serde_json::json!({
            "needs_wizard": needs_wizard,
            "sources_count": sources_count,
            "ollama": {
                "reachable": readiness.reachable,
                "base_url": ollama_base,
                "missing_models": readiness.missing_models
            },
            "answer_provider": answer_provider,
            "answer_ready": answer_ready,
            // R45.6: RAM + a sized answer model for the wizard's local picker.
            "machine": crate::machine::machine_json(),
            "sample": {
                "available": sample_available,
                "path": sample_path
            }
        })
    })
    .await
    .unwrap();
    Json(result)
}

#[allow(dead_code)]
#[derive(Serialize)]
struct LogFileOut {
    name: String,
    size: u64,
    modified: String,
}

pub async fn logs_handler(State(state): State<AppState>) -> Json<Value> {
    let data_dir = state.data_dir.clone();
    let result = tokio::task::spawn_blocking(move || {
        let dir = PathBuf::from(&data_dir);
        let mut out = vec![];
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
                // regex ^anchorcore\.log(\.\d+)?$
                let is_log = name == "anchorcore.log" || (name.starts_with("anchorcore.log.") && name["anchorcore.log.".len()..].chars().all(|c| c.is_ascii_digit()));
                if !is_log {
                    continue;
                }
                if let Ok(meta) = entry.metadata() {
                    let size = meta.len();
                    let modified = meta.modified().ok().and_then(|t| {
                        let dt: chrono::DateTime<chrono::Utc> = t.into();
                        Some(dt.to_rfc3339())
                    }).unwrap_or_default();
                    out.push(serde_json::json!({"name": name, "size": size, "modified": modified}));
                }
            }
        }
        out.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
        Value::Array(out)
    })
    .await
    .unwrap();
    Json(result)
}

pub async fn log_download_handler(
    State(state): State<AppState>,
    Path(filename): Path<String>,
) -> Result<axum::response::Response, (StatusCode, Json<Value>)> {
    // Validate regex ^anchorcore\.log(\.\d+)?$
    let is_valid = filename == "anchorcore.log"
        || (filename.starts_with("anchorcore.log.") && filename["anchorcore.log.".len()..].chars().all(|c| c.is_ascii_digit()));
    if !is_valid || filename.contains('/') || filename.contains('\\') || filename.contains("..") {
        return Err((StatusCode::NOT_FOUND, Json(serde_json::json!({"detail": "Log file not found"}))));
    }
    let data_dir = state.data_dir.clone();
    let path = PathBuf::from(&data_dir).join(&filename);
    if !path.is_file() {
        return Err((StatusCode::NOT_FOUND, Json(serde_json::json!({"detail": "Log file not found"}))));
    }
    let data = tokio::fs::read(&path).await.map_err(|_| (StatusCode::NOT_FOUND, Json(serde_json::json!({"detail": "Log file not found"}))))?;
    // R10.8: bound log download — cap at 512KB to avoid serving huge files
    const MAX_LOG_BYTES: usize = 512 * 1024;
    let (data, truncated) = if data.len() > MAX_LOG_BYTES {
        (data[..MAX_LOG_BYTES].to_vec(), true)
    } else {
        (data, false)
    };
    let mut resp = axum::response::Response::new(axum::body::Body::from(data));
    resp.headers_mut().insert("content-type", "text/plain".parse().unwrap());
    resp.headers_mut().insert("content-disposition", format!("attachment; filename=\"{}\"", filename).parse().unwrap());
    if truncated {
        resp.headers_mut().insert("x-truncated", "true".parse().unwrap());
        resp.headers_mut().insert("x-truncated-limit", MAX_LOG_BYTES.to_string().parse().unwrap());
    }
    Ok(resp)
}

pub fn find_ollama_binary() -> Option<std::path::PathBuf> {    // find binary
    let candidates = [
        "/opt/homebrew/bin/ollama",
        "/usr/local/bin/ollama",
        "/Applications/Ollama.app/Contents/Resources/ollama",
        "/usr/bin/ollama",
    ];
    for p in candidates {
        if std::path::Path::new(p).exists() {
            return Some(p.into());
        }
    }
    if let Ok(out) = std::process::Command::new("which").arg("ollama").output() {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() && std::path::Path::new(&s).exists() {
                return Some(s.into());
            }
        }
    }
    if std::process::Command::new("ollama").arg("--version").output().map(|o| o.status.success()).unwrap_or(false) {
        return Some("ollama".into());
    }
    None
}

pub async fn poll_ollama_up(probe_url: &str, attempts: u32) -> bool {
    // poll in 500ms steps
    for _ in 0..attempts {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        if let Ok(c) = reqwest::Client::builder().timeout(std::time::Duration::from_secs(1)).build() {
            if let Ok(r) = c.get(probe_url).send().await {
                if r.status().is_success() {
                    return true;
                }
            }
        }
    }
    false
}

pub async fn ollama_start_handler(State(state): State<AppState>) -> Json<Value> {
    let base = state.settings.get("ollama_base_url", None).unwrap_or_else(|| "http://localhost:11434".to_string());
    if ollama_probe_disabled(&base) {
        return Json(serde_json::json!({"ok": false, "error": "ollama probe disabled for tests (localhost:1)"}));
    }
    // quick probe
    let probe_url = format!("{}/api/tags", base.trim_end_matches('/'));
    if probe_ollama(&base, 2).await {
        return Json(serde_json::json!({"ok": true, "already_running": true}));
    }
    let Some(path) = find_ollama_binary() else {
        return Json(serde_json::json!({"ok": false, "error": "ollama not found — install from https://ollama.com"}));
    };
    tracing::info!("ollama start requested via API, launching {:?} serve", path);
    let _ = std::process::Command::new(&path)
        .arg("serve")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .stdin(std::process::Stdio::null())
        .spawn();
    if poll_ollama_up(&probe_url, 16).await {
        return Json(serde_json::json!({"ok": true, "launched": true}));
    }
    Json(serde_json::json!({"ok": false, "error": "ollama launched but not reachable at ".to_string() + &base, "launched": true}))
}

// ── In-app update check (R29.1) ─────────────────────────────────────────────
//
// Local-first rules encoded here:
// - a plain `GET /system/update` NEVER touches the network — it only returns
//   the cached result (or "not checked yet"), so opening the UI cannot phone
//   home;
// - the GitHub request happens only when the user asked (`?refresh=1`, the
//   Check button) or when startup checks were explicitly opted into
//   (`update_check_enabled` / `ANCHOR_UPDATE_CHECK_ENABLED`);
// - the request carries nothing but the fetch itself (fixed URL, our own
//   User-Agent) and failures are reported, never retried behind the user's back.
const GITHUB_LATEST_RELEASE: &str = "https://api.github.com/repos/Atsirkunov/anchorcore/releases/latest";
const RELEASES_PAGE: &str = "https://github.com/Atsirkunov/anchorcore/releases/latest";
/// Rapid-click floor: never hit GitHub more than once a minute even if the
/// button is mashed (unauthenticated GitHub allows 60 requests/hour/IP).
const UPDATE_MIN_INTERVAL_SECS: u64 = 60;

#[derive(Serialize, Clone, Debug)]
pub struct UpdateStatus {
    pub current: String,
    pub latest: Option<String>,
    pub update_available: bool,
    pub release_url: Option<String>,
    pub published_at: Option<String>,
    pub asset_url: Option<String>,
    pub checked_at: Option<String>,
    pub check_enabled: bool,
    pub error: Option<String>,
}

struct CachedUpdate {
    status: UpdateStatus,
    at: std::time::Instant,
}

static UPDATE_CACHE: std::sync::OnceLock<std::sync::Mutex<Option<CachedUpdate>>> = std::sync::OnceLock::new();

fn update_cache() -> &'static std::sync::Mutex<Option<CachedUpdate>> {
    UPDATE_CACHE.get_or_init(|| std::sync::Mutex::new(None))
}

/// `1` / `true` / `yes` / `on` are on; everything else (incl. `0`, empty,
/// unset) is off. Mirrors the Settings `''`-means-unset semantics.
fn truthy_setting(v: Option<String>) -> bool {
    matches!(v.as_deref().map(|s| s.trim().to_ascii_lowercase()).as_deref(), Some("1" | "true" | "yes" | "on"))
}

fn update_check_enabled(settings: &crate::settings::SettingsService) -> bool {
    truthy_setting(settings.get("update_check_enabled", None))
}

/// `1.2.3` / `v1.2.3` / `1.2.3-rc1` → `(1, 2, 3)` — numeric compare, so
/// `1.0.9` is older than `1.0.20` (string compare gets that wrong).
fn parse_version(v: &str) -> (u64, u64, u64) {
    let core = v.trim().trim_start_matches('v');
    let core = core.split(['-', '+']).next().unwrap_or(core);
    let mut parts = core.split('.').map(|p| p.parse::<u64>().unwrap_or(0));
    (parts.next().unwrap_or(0), parts.next().unwrap_or(0), parts.next().unwrap_or(0))
}

fn is_newer(latest: &str, current: &str) -> bool {
    parse_version(latest) > parse_version(current)
}

fn update_asset_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "AnchorCore-windows.zip"
    } else if cfg!(target_os = "macos") {
        "AnchorCore-macos.zip"
    } else {
        "AnchorCore-linux.zip"
    }
}

fn update_url() -> String {
    // Tests/mirrors can point this at a stub; never a user-facing setting.
    std::env::var("ANCHOR_UPDATE_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| GITHUB_LATEST_RELEASE.to_string())
}

fn idle_update_status(current: &str, enabled: bool) -> UpdateStatus {
    UpdateStatus {
        current: current.to_string(),
        latest: None,
        update_available: false,
        release_url: Some(RELEASES_PAGE.to_string()),
        published_at: None,
        asset_url: None,
        checked_at: None,
        check_enabled: enabled,
        error: None,
    }
}

async fn fetch_latest_release() -> Result<(String, Option<String>, Option<String>), String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .user_agent(format!("anchorcore/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| format!("http client: {e}"))?;
    let resp = client
        .get(update_url())
        .header("accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|_| "couldn't reach github.com (offline?)".to_string())?;
    if !resp.status().is_success() {
        return Err(format!("github.com returned HTTP {}", resp.status().as_u16()));
    }
    let body: Value = resp.json().await.map_err(|_| "unexpected response from github.com".to_string())?;
    let tag = body.get("tag_name").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    if tag.is_empty() {
        return Err("github.com response carried no tag_name".to_string());
    }
    let html = body.get("html_url").and_then(|v| v.as_str()).map(str::to_string);
    let published = body.get("published_at").and_then(|v| v.as_str()).map(str::to_string);
    Ok((tag, html, published))
}

async fn check_for_update(enabled: bool, current: &str) -> UpdateStatus {
    let checked_at = chrono::Utc::now().to_rfc3339();
    match fetch_latest_release().await {
        Ok((tag, html, published)) => {
            let latest = tag.trim_start_matches('v').to_string();
            UpdateStatus {
                current: current.to_string(),
                update_available: is_newer(&latest, current),
                release_url: Some(html.unwrap_or_else(|| RELEASES_PAGE.to_string())),
                published_at: published,
                asset_url: Some(format!("{RELEASES_PAGE}/download/{}", update_asset_name())),
                latest: Some(latest),
                checked_at: Some(checked_at),
                check_enabled: enabled,
                error: None,
            }
        }
        Err(e) => UpdateStatus {
            error: Some(e),
            checked_at: Some(checked_at),
            ..idle_update_status(current, enabled)
        },
    }
}

fn cached_update_clone(enabled: bool) -> Option<UpdateStatus> {
    let guard = update_cache().lock().unwrap();
    guard.as_ref().map(|c| {
        let mut s = c.status.clone();
        s.check_enabled = enabled;
        s
    })
}

#[derive(Deserialize)]
pub struct UpdateQuery {
    /// `refresh=1` performs the GitHub request; anything else reads the cache.
    pub refresh: Option<String>,
}

/// `GET /system/update` — cached result only unless `refresh=1`. The opt-in
/// startup check (`spawn_update_check`) fills the same cache.
pub async fn update_handler(State(state): State<AppState>, Query(q): Query<UpdateQuery>) -> Json<UpdateStatus> {
    let enabled = update_check_enabled(&state.settings);
    let current = env!("CARGO_PKG_VERSION");
    if !truthy_setting(q.refresh) {
        return Json(cached_update_clone(enabled).unwrap_or_else(|| idle_update_status(current, enabled)));
    }
    let recent = {
        let guard = update_cache().lock().unwrap();
        guard.as_ref().map(|c| c.at.elapsed() < std::time::Duration::from_secs(UPDATE_MIN_INTERVAL_SECS)).unwrap_or(false)
    };
    if recent {
        if let Some(s) = cached_update_clone(enabled) {
            return Json(s);
        }
    }
    let status = check_for_update(enabled, current).await;
    *update_cache().lock().unwrap() = Some(CachedUpdate { status: status.clone(), at: std::time::Instant::now() });
    Json(status)
}

/// Startup check — only runs when `update_check_enabled` was opted in. One
/// background request per launch; the result lands in the cache the UI reads.
/// Never blocks boot, never retries; failures only log.
pub fn spawn_update_check(state: AppState) {
    if !update_check_enabled(&state.settings) {
        return;
    }
    tokio::spawn(async move {
        let status = check_for_update(true, env!("CARGO_PKG_VERSION")).await;
        if let Some(err) = &status.error {
            tracing::warn!("startup update check failed: {}", err);
        } else if status.update_available {
            tracing::info!("newer release available: {} (current {})", status.latest.as_deref().unwrap_or("?"), status.current);
        }
        *update_cache().lock().unwrap() = Some(CachedUpdate { status, at: std::time::Instant::now() });
    });
}

// ── App-served agent skills (R23.3 setup-only, generalized by R23.6) ─────────
//
// Release zips ship only the binary, so zip users have no skill files on
// disk — the app serves its own SKILL.md files embedded at compile time
// (`include_str!`, same mechanism as the SQL migrations in `db.rs`), so an
// agent can fetch setup + search over HTTP with no git clone. One mechanism:
// `GET /system/skills/:name` (the R23.3 `/system/skill` shape never shipped,
// so no compat alias).
const SETUP_SKILL_MD: &str = include_str!("../../../../.agents/skills/anchorcore-setup/SKILL.md");
const SEARCH_SKILL_MD: &str = include_str!("../../../../.agents/skills/anchorcore-search/SKILL.md");

fn skill_markdown(name: &str) -> Option<&'static str> {
    match name {
        "setup" | "anchorcore-setup" => Some(SETUP_SKILL_MD),
        "search" | "anchorcore-search" => Some(SEARCH_SKILL_MD),
        _ => None,
    }
}

/// `GET /system/skills/:name` — one embedded skill as `text/markdown`;
/// unknown names 404 with the known list.
pub async fn skills_handler(
    Path(name): Path<String>,
) -> Result<([(axum::http::header::HeaderName, &'static str); 1], &'static str), (StatusCode, Json<Value>)> {
    match skill_markdown(name.trim()) {
        Some(md) => Ok(([(axum::http::header::CONTENT_TYPE, "text/markdown; charset=utf-8")], md)),
        None => Err((StatusCode::NOT_FOUND, Json(serde_json::json!({"detail": format!("Unknown skill '{name}' (setup, search)")}))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedder::Embedder;
    use crate::jobs::JobManager;
    use crate::secrets::SecretStore;
    use crate::settings::SettingsService;
    use std::sync::Arc;

    fn sorted_keys(v: &Value) -> Vec<String> {
        let mut keys: Vec<String> = v.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        keys
    }

    /// Rust-side replacement for the frozen `test_onboarding_state`
    /// (deselected from rust-conformance in ffd3e21 after B54 added
    /// `answer_ready`): pins the exact `/system/onboarding` key set so any
    /// future addition or rename fails loudly here instead of drifting.
    #[tokio::test]
    async fn onboarding_response_shape_is_versioned() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("data");
        std::fs::create_dir_all(&data_dir).unwrap();
        let store = SecretStore::new(dir.path().join("secrets.enc"));
        let settings = Arc::new(SettingsService::new(store));
        settings.set_data_dir(&data_dir);
        let embedder = Arc::new(Embedder::new(settings.clone()));
        let state = AppState {
            data_dir: data_dir.to_string_lossy().to_string(),
            settings,
            jobs: JobManager::new(),
            csrf_token: "test".to_string(),
            embedder,
        };
        let res = onboarding_handler(State(state)).await;
        let v = res.0;
        assert_eq!(
            sorted_keys(&v),
            ["answer_provider", "answer_ready", "machine", "needs_wizard", "ollama", "sample", "sources_count"]
        );
        assert_eq!(sorted_keys(&v["machine"]), ["answer_models", "recommended_answer_model", "total_memory_bytes"]);
        assert_eq!(sorted_keys(&v["ollama"]), ["base_url", "missing_models", "reachable"]);
        assert_eq!(sorted_keys(&v["sample"]), ["available", "path"]);
        assert_eq!(v["needs_wizard"], true);
        assert_eq!(v["sources_count"], 0);
        assert!(v["answer_ready"].is_boolean(), "answer_ready must be a bool: {v}");
    }

    #[test]
    fn version_compare_is_numeric_not_lexicographic() {
        assert!(is_newer("1.0.20", "1.0.9"));
        assert!(is_newer("v1.0.21", "1.0.20"));
        assert!(is_newer("2.0.0", "1.9.9"));
        assert!(is_newer("1.1", "1.0.99"));
        assert!(!is_newer("1.0.21", "1.0.21"));
        assert!(!is_newer("1.0.20", "1.0.21"));
        assert_eq!(parse_version("v1.0.21-rc1"), (1, 0, 21));
    }

    #[test]
    fn truthy_setting_accepts_common_spellings() {
        for on in ["1", "true", "YES", " on "] {
            assert!(truthy_setting(Some(on.to_string())), "{on} should be on");
        }
        for off in [None, Some(String::new()), Some("0".to_string()), Some("false".to_string())] {
            assert!(!truthy_setting(off.clone()), "{off:?} should be off");
        }
    }

    fn repo_skill_bytes(name: &str) -> Vec<u8> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../../.agents/skills/{name}/SKILL.md"));
        std::fs::read(&path).unwrap()
    }

    async fn get_skill(name: &str) -> axum::response::Response {
        match skills_handler(Path(name.to_string())).await {
            Ok(ok) => axum::response::IntoResponse::into_response(ok),
            Err(err) => axum::response::IntoResponse::into_response(err),
        }
    }

    async fn assert_skill_byte_identical(name: &str) {
        let resp = get_skill(name).await;
        assert_eq!(resp.status(), StatusCode::OK, "{name} should serve");
        assert_eq!(resp.headers()["content-type"], "text/markdown; charset=utf-8");
        let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
        assert_eq!(body.as_ref(), skill_markdown(name).unwrap().as_bytes());
        assert_eq!(body.as_ref(), repo_skill_bytes(&format!("anchorcore-{name}")).as_slice());
    }

    /// R23.3/R23.6: each served skill is byte-identical to its repo file.
    #[tokio::test]
    async fn skills_endpoint_serves_setup_skill_byte_identical() {
        assert_skill_byte_identical("setup").await;
    }

    #[tokio::test]
    async fn skills_endpoint_serves_search_skill_byte_identical() {
        assert_skill_byte_identical("search").await;
    }

    /// R23.6: unknown skill names 404 (no compat alias — `/system/skill`
    /// never shipped, so nothing else may resolve).
    #[tokio::test]
    async fn skills_endpoint_unknown_name_is_404() {
        for name in ["nope", "skill", "SETUP", "setup.md"] {
            let resp = get_skill(name).await;
            assert_eq!(resp.status(), StatusCode::NOT_FOUND, "{name} should 404");
            let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
            let v: Value = serde_json::from_slice(&body).unwrap();
            assert!(v["detail"].as_str().unwrap().contains("setup"), "detail names setup: {v}");
        }
    }

    /// The trust contract: a plain GET must never perform the GitHub request.
    #[tokio::test]
    async fn plain_update_get_never_checks_network() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("data");
        std::fs::create_dir_all(&data_dir).unwrap();
        let store = SecretStore::new(dir.path().join("secrets.enc"));
        let settings = Arc::new(SettingsService::new(store));
        settings.set_data_dir(&data_dir);
        let embedder = Arc::new(Embedder::new(settings.clone()));
        let state = AppState {
            data_dir: data_dir.to_string_lossy().to_string(),
            settings,
            jobs: JobManager::new(),
            csrf_token: "test".to_string(),
            embedder,
        };
        let res = update_handler(State(state), Query(UpdateQuery { refresh: None })).await;
        let v = res.0;
        assert!(v.checked_at.is_none(), "plain GET must not perform a check: {v:?}");
        assert!(v.latest.is_none());
        assert!(!v.update_available);
    }
}
