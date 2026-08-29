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
    base.starts_with("http://localhost") || base.starts_with("http://127.0.0.1")
}

pub(crate) fn ollama_probe_disabled(base: &str) -> bool {
    base == "http://localhost:1" || base == "http://127.0.0.1:1" || base == "http://localhost:1/" || base == "http://127.0.0.1:1/"
}

pub(crate) async fn probe_ollama(base: &str, timeout_secs: u64) -> bool {
    // real probe like health.rs (was heuristic localhost:1 and always false)
    let url = format!("{}/api/tags", base.trim_end_matches('/'));
    match reqwest::Client::builder().timeout(std::time::Duration::from_secs(timeout_secs)).build() {
        Ok(c) => c.get(&url).send().await.map(|r| r.status().is_success()).unwrap_or(false),
        Err(_) => false,
    }
}

struct StatusSnapshot {
    ollama_base: String,
    classifier_model: String,
    embed_model: String,
    classifier_base: String,
    embed_base: String,
    answer_model: String,
    answer_base: String,
    answer_api_key: String,
    answer_provider: String,
}

fn status_snapshot(settings: &crate::settings::SettingsService) -> StatusSnapshot {
    let ollama_base = settings.get("ollama_base_url", None).unwrap_or_else(|| "http://localhost:11434".to_string());
    let classifier_model = settings.get("classifier_model", None).unwrap_or_else(|| "llama3.2:3b".to_string());
    let embed_model = settings.get("embed_model", None).unwrap_or_else(|| "nomic-embed-text".to_string());
    let classifier_base = settings
        .get("classifier_base_url", None)
        .or_else(|| settings.get("ollama_base_url", None))
        .unwrap_or_else(|| "http://localhost:11434".to_string());
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
        classifier_model,
        embed_model,
        classifier_base,
        embed_base,
        answer_model,
        answer_base,
        answer_api_key,
        answer_provider,
    }
}

pub async fn status_handler(State(state): State<AppState>) -> Json<Value> {
    let data_dir = state.data_dir.clone();
    // capture settings values before blocking (use cached snapshot without DB conn for speed)
    let snap = status_snapshot(&state.settings);
    let ollama_reachable = if ollama_probe_disabled(&snap.ollama_base) {
        false
    } else {
        probe_ollama(&snap.ollama_base, 3).await
    };
    let classifier_is_local = provider_is_local(&snap.classifier_base);
    let embed_is_local = provider_is_local(&snap.embed_base);
    let result = tokio::task::spawn_blocking(move || {
        let db_path = crate::db::resolve_db_path(&data_dir);
        let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| rusqlite::Connection::open(&db_path).unwrap());
        let pending: i64 = conn.query_row("SELECT COUNT(*) FROM chunks WHERE embedding IS NULL", [], |r| r.get(0)).unwrap_or(0);
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
                "reachable": ollama_reachable,
                "base_url": snap.ollama_base,
                "classifier_model": snap.classifier_model,
                "embed_model": snap.embed_model,
                "missing_models": []
            },
            "classifier": {
                "provider": if classifier_is_local { "local" } else { "cloud" },
                "base_url": snap.classifier_base,
                "model": snap.classifier_model,
                "concurrency": 4
            },
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
            },
            "answer": {
                "provider": snap.answer_provider,
                "model": snap.answer_model,
                "base_url": snap.answer_base,
            },
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

pub async fn onboarding_handler(State(state): State<AppState>) -> Json<Value> {
    let data_dir = state.data_dir.clone();
    let settings = state.settings.clone();
    let result = tokio::task::spawn_blocking(move || {
        let db_path = crate::db::resolve_db_path(&data_dir);
        let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| rusqlite::Connection::open(&db_path).unwrap());
        let sources_count: i64 = conn.query_row("SELECT COUNT(*) FROM sources", [], |r| r.get(0)).unwrap_or(0);
        let needs_wizard = sources_count == 0;
        let ollama_base = settings.get("ollama_base_url", None).unwrap_or_else(|| "http://localhost:11434".to_string());
        let ollama_reachable = !(ollama_base == "http://localhost:1" || ollama_base == "http://127.0.0.1:1" || ollama_base == "http://localhost:1/" || ollama_base == "http://127.0.0.1:1/");
        let answer_base = settings.get("answer_base_url", None).unwrap_or_else(|| "https://api.openai.com/v1".to_string());
        let answer_api_key = settings.get("answer_api_key", None).unwrap_or_default();
        let answer_provider = if answer_base.starts_with("http://localhost") || answer_base.starts_with("http://127.0.0.1") {
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
                "reachable": ollama_reachable,
                "base_url": ollama_base,
                "missing_models": []
            },
            "answer_provider": answer_provider,
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

pub(crate) fn find_ollama_binary() -> Option<std::path::PathBuf> {    // find binary
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

pub(crate) async fn poll_ollama_up(probe_url: &str, attempts: u32) -> bool {
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
