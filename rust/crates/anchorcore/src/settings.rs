use crate::secrets::SecretStore;
use rusqlite::Connection;
use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};
use std::time::{Duration, Instant};

pub const SETTING_KEYS: &[&str] = &[
    "ollama_base_url",
    "embed_model",
    "embed_timeout",
    "embed_base_url",
    "answer_model",
    "answer_base_url",
    "answer_timeout",
    "answer_reasoning_effort",
    "tag_reuse_threshold",
    "update_check_enabled",
    "extract_entities",
    "llm_mode",
    "acronym_expansion",
    "asset_max_bytes",
    "embed_provider",
];
pub const SECRET_KEYS: &[&str] = &["answer_api_key", "embed_api_key"];
pub const ALL_KEYS: &[&str] = &[
    "ollama_base_url",
    "embed_model",
    "embed_timeout",
    "embed_base_url",
    "answer_model",
    "answer_base_url",
    "answer_timeout",
    "answer_reasoning_effort",
    "tag_reuse_threshold",
    "update_check_enabled",
    "extract_entities",
    "llm_mode",
    "acronym_expansion",
    "asset_max_bytes",
    "embed_provider",
    "answer_api_key",
    "embed_api_key",
];
pub const SECRET_PREFIX: &str = "app:";
/// R45.18: secrets of deleted settings, wiped at startup so a key the UI
/// can no longer show or clear doesn't linger in the keychain.
const RETIRED_SECRET_KEYS: &[&str] = &["classifier_api_key"];

/// R45.2: who writes answers. `agent` — the user's AI agent (MCP or the
/// REST skill) composes from cited passages and the server runs no chat
/// model on the answer path; `local` — the configured answer model
/// composes (today's behavior, and the default so upgrades don't change).
pub const LLM_MODE_AGENT: &str = "agent";
pub const LLM_MODE_LOCAL: &str = "local";

/// Effective `llm_mode`: anything but `agent` reads as `local`.
pub fn llm_mode(settings: &SettingsService) -> &'static str {
    match settings.get("llm_mode", None) {
        Some(v) if v.trim().eq_ignore_ascii_case(LLM_MODE_AGENT) => LLM_MODE_AGENT,
        _ => LLM_MODE_LOCAL,
    }
}

pub fn is_agent_mode(settings: &SettingsService) -> bool {
    llm_mode(settings) == LLM_MODE_AGENT
}

/// R45.4: `embed_provider` values (`auto` = the HTTP embedder when it
/// serves, else the built-in model once downloaded — `embedder::resolve_provider`).
pub const EMBED_PROVIDERS: &[&str] = &["auto", "builtin", "ollama"];

/// R45.2/R45.4: reject values the effective read would silently coerce.
fn invalid_value(key: &str, value: &Value) -> Option<String> {
    let allowed: &[&str] = match key {
        "llm_mode" => &[LLM_MODE_AGENT, LLM_MODE_LOCAL],
        "embed_provider" => EMBED_PROVIDERS,
        _ => return None,
    };
    let v = value.as_str().map(|s| s.trim().to_lowercase()).unwrap_or_default();
    if v.is_empty() || allowed.contains(&v.as_str()) {
        return None;
    }
    let quoted: Vec<String> = allowed.iter().map(|a| format!("'{a}'")).collect();
    Some(format!("{key} must be one of {}", quoted.join(", ")))
}
const CACHE_TTL: Duration = Duration::from_secs(3);

// Test-only Ollama URL: the test harness points Ollama at a dead port so
// degradation paths are exercised. A real launch must never honor it —
// if it leaks in via env/.env/DB (stray export, `open` env inheritance), fall
// back to the real default and warn. Tests opt in with ANCHOR_ALLOW_TEST_URLS=1.
fn is_test_sentinel_url(value: &str) -> bool {
    let v = value.trim_end_matches('/');
    v == "http://localhost:1" || v == "http://127.0.0.1:1"
}

fn test_urls_allowed() -> bool {
    std::env::var("ANCHOR_ALLOW_TEST_URLS").as_deref() == Ok("1")
}

fn guard_test_ollama_url(value: Option<String>) -> Option<String> {
    match value {
        Some(v) if is_test_sentinel_url(&v) && !test_urls_allowed() => {
            tracing::warn!(
                "ignoring test-only ollama_base_url '{}' (set ANCHOR_ALLOW_TEST_URLS=1 to allow); using http://localhost:11434",
                v
            );
            Some("http://localhost:11434".to_string())
        }
        other => other,
    }
}

#[allow(dead_code)]
static ENV_CACHE: OnceLock<HashMap<String, String>> = OnceLock::new();

fn env_value(key: &str) -> Option<String> {
    // ANCHOR_ prefix, like Python Settings env_prefix
    let env_key = format!("ANCHOR_{}", key.to_uppercase());
    if let Ok(v) = std::env::var(&env_key) {
        if !v.trim().is_empty() {
            return Some(v);
        }
    }
    // also check .env via dotenvy (loaded once)
    let _ = dotenvy::dotenv();
    if let Ok(v) = std::env::var(&env_key) {
        if !v.trim().is_empty() {
            return Some(v);
        }
    }
    // fallback to Settings defaults (mirrors config.py defaults)
    match key {
        "ollama_base_url" => Some("http://localhost:11434".to_string()),
        "embed_model" => Some("nomic-embed-text".to_string()),
        "answer_model" => Some("gpt-4o-mini".to_string()),
        "answer_base_url" => Some("https://api.openai.com/v1".to_string()),
        "embed_base_url" => None,
        "answer_reasoning_effort" => Some("none".to_string()),
        "tag_reuse_threshold" => Some("0.82".to_string()),
        // R46.7: provisional per-image cap (20 MB); R46.14 sets it from data.
        "asset_max_bytes" => Some(crate::assets::DEFAULT_MAX_BYTES.to_string()),
        // R45.1/R45.18: ingest never calls a chat model.
        "extract_entities" => Some("rules".to_string()),
        "llm_mode" => Some(LLM_MODE_LOCAL.to_string()),
        "embed_provider" => Some("auto".to_string()),
        _ => None,
    }
}

/// Empty/whitespace values mean "unset": the Settings UI stores `''` for
/// blank fields, and Python's `db or env or default` chain treated them as
/// falsy. Returning `None` keeps `.or_else(fallback)` chains (base URLs,
/// models) working regardless of whether the 3s cache is warm or cold.
fn non_empty(v: Option<String>) -> Option<String> {
    v.filter(|s| !s.trim().is_empty())
}

pub struct SettingsService {
    secrets: SecretStore,
    cache: RwLock<HashMap<String, (String, Instant)>>,
    /// Canonical data dir for DB-backed settings reads. Without it, cached_get
    /// falls back to ANCHOR_DATA_DIR/CWD — the wrong DB whenever the app runs
    /// with --data-dir (or ~/.anchorcore) and the env var is unset, making
    /// saved settings (answer model, provider trust) invisible to ask/search.
    data_dir: RwLock<Option<std::path::PathBuf>>,
}

impl SettingsService {
    pub fn new(secrets: SecretStore) -> Self {
        Self {
            secrets,
            cache: RwLock::new(HashMap::new()),
            data_dir: RwLock::new(None),
        }
    }

    /// Pin the data dir for DB-backed reads (called once at startup).
    pub fn set_data_dir(&self, dir: &std::path::Path) {
        *self.data_dir.write().unwrap() = Some(dir.to_path_buf());
        self.cache.write().unwrap().clear();
    }

    /// R45.18: drop stored secrets of retired settings (idempotent).
    pub fn forget_retired_secrets(&self) {
        for key in RETIRED_SECRET_KEYS {
            let skey = format!("{}{}", SECRET_PREFIX, key);
            if self.secrets.get(&skey).is_some() {
                self.secrets.delete(&skey);
            }
        }
    }

    /// The pinned data dir, if `set_data_dir` ran (R45.5 records query
    /// fingerprints there; unpinned test services skip it).
    pub fn data_dir(&self) -> Option<std::path::PathBuf> {
        self.data_dir.read().unwrap().clone()
    }

    pub fn get(&self, key: &str, conn: Option<&Connection>) -> Option<String> {
        if !SETTING_KEYS.contains(&key) && !SECRET_KEYS.contains(&key) {
            return env_value(key);
        }
        if SECRET_KEYS.contains(&key) {
            let skey = format!("{}{}", SECRET_PREFIX, key);
            return non_empty(self.secrets.get(&skey)).or_else(|| env_value(key));
        }
        // non-secret: check DB first if conn provided, else cache
        if let Some(c) = conn {
            let resolved = non_empty(db_value(c, key).or_else(|| env_value(key)));
            return if key == "ollama_base_url" {
                guard_test_ollama_url(resolved)
            } else {
                resolved
            };
        }
        self.cached_get(key)
    }

    pub fn get_float(&self, key: &str, default: f64) -> f64 {
        self.get(key, None)
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(default)
    }

    pub fn snapshot(&self, conn: Option<&Connection>) -> serde_json::Value {
        let mut out = serde_json::Map::new();
        for &k in SETTING_KEYS.iter().chain(SECRET_KEYS.iter()) {
            let v = self.get(k, conn);
            if SECRET_KEYS.contains(&k) {
                out.insert(k.to_string(), serde_json::Value::String(if v.is_some() { "***set***".to_string() } else { "".to_string() }));
            } else {
                out.insert(k.to_string(), serde_json::Value::String(v.unwrap_or_default()));
            }
        }
        serde_json::Value::Object(out)
    }

    pub fn set(&self, key: &str, value: &str, conn: &Connection) -> Result<(), String> {
        let v = value.trim();
        if !SETTING_KEYS.contains(&key) && !SECRET_KEYS.contains(&key) {
            tracing::warn!("ignoring unknown setting key '{}'", key);
            return Ok(());
        }
        if SECRET_KEYS.contains(&key) {
            let skey = format!("{}{}", SECRET_PREFIX, key);
            if v.is_empty() {
                self.secrets.delete(&skey);
            } else {
                self.secrets.set(&skey, v);
            }
            self.invalidate(key);
            tracing::info!("setting {} updated (secret)", key);
            return Ok(());
        }
        if v.is_empty() {
            // Empty means "unset": drop the override row so reads fall back to
            // env/defaults. Mirrors the secret branch above and Python's
            // falsy resolution — an empty row used to break base-URL fallbacks.
            conn.execute("DELETE FROM app_settings WHERE key = ?1", rusqlite::params![key])
                .map_err(|e| e.to_string())?;
            self.invalidate(key);
            tracing::info!("setting {} cleared (empty)", key);
            return Ok(());
        }
        // DB override
        let exists: bool = conn
            .query_row(
                "SELECT 1 FROM app_settings WHERE key = ?1",
                rusqlite::params![key],
                |_| Ok(()),
            )
            .is_ok();
        if exists {
            conn.execute(
                "UPDATE app_settings SET value = ?1 WHERE key = ?2",
                rusqlite::params![v, key],
            )
            .map_err(|e| e.to_string())?;
        } else {
            conn.execute(
                "INSERT INTO app_settings (key, value) VALUES (?1, ?2)",
                rusqlite::params![key, v],
            )
            .map_err(|e| e.to_string())?;
        }
        self.invalidate(key);
        tracing::info!("setting {} updated", key);
        Ok(())
    }

    pub fn clear(&self, key: &str, conn: &Connection) {
        if SECRET_KEYS.contains(&key) {
            self.secrets.delete(&format!("{}{}", SECRET_PREFIX, key));
        } else {
            let _ = conn.execute("DELETE FROM app_settings WHERE key = ?1", rusqlite::params![key]);
        }
        self.invalidate(key);
    }

    fn cached_get(&self, key: &str) -> Option<String> {
        {
            let cache = self.cache.read().unwrap();
            if let Some((v, exp)) = cache.get(key) {
                if Instant::now() < *exp {
                    return if v.is_empty() { None } else { Some(v.clone()) };
                }
            }
        }
        // need DB read; prefer the pinned data dir (set at startup), else the
        // legacy env/CWD fallback. Use open_db (WAL/busy_timeout/FK), not raw open.
        let data_dir = self
            .data_dir
            .read()
            .unwrap()
            .clone()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| std::env::var("ANCHOR_DATA_DIR").unwrap_or_else(|_| "data".to_string()));
        let db_path = crate::db::resolve_db_path(&data_dir);
        let conn = crate::db::open_db(&db_path).ok();
        let db_val = conn.as_ref().and_then(|c| db_value(c, key));
        let resolved = non_empty(db_val.or_else(|| env_value(key)));
        let resolved = if key == "ollama_base_url" {
            guard_test_ollama_url(resolved)
        } else {
            resolved
        };
        let mut cache = self.cache.write().unwrap();
        cache.insert(
            key.to_string(),
            (resolved.clone().unwrap_or_default(), Instant::now() + CACHE_TTL),
        );
        resolved
    }

    fn invalidate(&self, key: &str) {
        self.cache.write().unwrap().remove(key);
    }
}

fn db_value(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row(
        "SELECT value FROM app_settings WHERE key = ?1",
        rusqlite::params![key],
        |r| r.get::<_, String>(0),
    )
    .ok()
}

// --- HTTP handlers (R6.1) ---
use axum::{extract::State, http::StatusCode, Json};
use serde_json::Value;

use crate::health::AppState;

pub async fn get_handler(State(state): State<AppState>) -> Json<Value> {
    let data_dir = state.data_dir.clone();
    let settings = state.settings.clone();
    let result = tokio::task::spawn_blocking(move || {
        let db_path = crate::db::resolve_db_path(&data_dir);
        let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| Connection::open(&db_path).unwrap());
        settings.snapshot(Some(&conn))
    })
    .await
    .unwrap();
    Json(result)
}

pub async fn put_handler(
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let obj = match payload.as_object() {
        Some(m) => m.clone(),
        None => {
            return Err((
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(serde_json::json!({"detail": "invalid payload"})),
            ))
        }
    };
    let unknown: Vec<String> = obj.keys().filter(|k| !ALL_KEYS.contains(&k.as_str())).cloned().collect();
    if !unknown.is_empty() {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({"detail": format!("unknown settings: {}", unknown.join(", "))})),
        ));
    }
    if let Some(detail) = obj.iter().find_map(|(k, v)| if v.is_null() { None } else { invalid_value(k, v) }) {
        return Err((StatusCode::UNPROCESSABLE_ENTITY, Json(serde_json::json!({"detail": detail}))));
    }
    let data_dir = state.data_dir.clone();
    let settings = state.settings.clone();
    let result = tokio::task::spawn_blocking(move || {
        let db_path = crate::db::resolve_db_path(&data_dir);
        let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| Connection::open(&db_path).unwrap());
        for (k, v) in obj.iter() {
            if v.is_null() {
                settings.clear(k, &conn);
            } else {
                let s = if let Some(s) = v.as_str() {
                    s.to_string()
                } else {
                    v.to_string()
                };
                // trim quotes for JSON numbers/bools that were stringified with quotes?
                // For numbers/bools, v.to_string() yields e.g. "true" without quotes, correct.
                // For strings, we used as_str, so no extra quotes.
                let _ = settings.set(k, &s, &conn);
            }
        }
        settings.snapshot(Some(&conn))
    })
    .await
    .unwrap();
    Ok(Json(result))
}

pub async fn test_connection_handler(
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let provider = payload
        .get("provider")
        .and_then(|v| v.as_str())
        .unwrap_or("ollama")
        .to_string();
    match provider.as_str() {
        "ollama" => Ok(Json(test_ollama(&state.settings).await)),
        "embedder" => Ok(Json(test_embedder(&state.settings).await)),
        "embed" => Ok(Json(test_embedder(&state.settings).await)),
        "answer" => Ok(Json(test_answer(&state.settings).await)),
        _ => Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({"detail": format!("unknown provider: {}", provider)})),
        )),
    }
}

fn connect_timeout() -> std::time::Duration {
    // The test harness sets ANCHOR_HTTP_CONNECT_TIMEOUT=0.2 to keep the suite fast (Windows quirk)
    if let Ok(v) = std::env::var("ANCHOR_HTTP_CONNECT_TIMEOUT") {
        if let Ok(f) = v.parse::<f64>() {
            return std::time::Duration::from_secs_f64(f.clamp(0.05, 10.0));
        }
    }
    if let Ok(v) = std::env::var("ANCHOR_CONNECT_TIMEOUT") {
        if let Ok(f) = v.parse::<f64>() {
            return std::time::Duration::from_secs_f64(f.clamp(0.05, 10.0));
        }
    }
    std::time::Duration::from_secs(3)
}

async fn test_ollama(settings: &SettingsService) -> Value {
    let base = settings
        .get("ollama_base_url", None)
        .unwrap_or_else(|| "http://localhost:11434".to_string());
    let url = format!("{}/api/tags", base.trim_end_matches('/'));
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .connect_timeout(connect_timeout())
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return serde_json::json!({"ok": false, "provider": "ollama", "message": format!("unreachable: {}", e)})
        }
    };
    let resp = match client.get(&url).send().await {
        Ok(r) => r,
        Err(e) => {
            return serde_json::json!({"ok": false, "provider": "ollama", "message": format!("unreachable: {}", e)})
        }
    };
    if !resp.status().is_success() {
        return serde_json::json!({"ok": false, "provider": "ollama", "message": format!("unreachable: status {}", resp.status())});
    }
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    let models: std::collections::HashSet<String> = body
        .get("models")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|m| m.get("name").and_then(|n| n.as_str()).map(|n| n.split(':').next().unwrap_or(n).to_string()))
                .collect()
        })
        .unwrap_or_default();
    let embed = settings
        .get("embed_model", None)
        .unwrap_or_else(|| "nomic-embed-text".to_string())
        .split(':')
        .next()
        .unwrap_or("nomic-embed-text")
        .to_string();
    let missing: Vec<String> = [embed]
        .into_iter()
        .filter(|m| !models.contains(m))
        .collect();
    if !missing.is_empty() {
        return serde_json::json!({"ok": false, "provider": "ollama", "message": format!("reachable; models missing: {} (run: ollama pull {})", missing.join(", "), missing.join(" && ollama pull "))});
    }
    serde_json::json!({"ok": true, "provider": "ollama", "message": "Ollama up, models present"})
}

async fn test_embedder(settings: &SettingsService) -> Value {
    let base = settings
        .get("embed_base_url", None)
        .or_else(|| settings.get("ollama_base_url", None))
        .unwrap_or_else(|| "http://localhost:11434".to_string());
    let is_local = crate::common::provider_is_local(&base);
    let api_key = settings.get("embed_api_key", None).unwrap_or_default();
    if !is_local && api_key.is_empty() {
        return serde_json::json!({"ok": false, "provider": "embedder", "message": "API key required for a cloud embedding provider"});
    }
    let model = settings.get("embed_model", None).unwrap_or_else(|| "nomic-embed-text".to_string());
    let url = format!("{}/v1/embeddings", base.trim_end_matches('/'));
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .connect_timeout(connect_timeout())
        .build()
    {
        Ok(c) => c,
        Err(e) => return serde_json::json!({"ok": false, "provider": "embedder", "message": format!("request failed: {}", e)}),
    };
    let mut req = client.post(&url).json(&serde_json::json!({
        "model": model,
        "input": ["ping"]
    }));
    if !api_key.is_empty() {
        req = req.header("Authorization", format!("Bearer {}", api_key));
    }
    match req.send().await {
        Ok(r) if r.status().is_success() => serde_json::json!({"ok": true, "provider": "embedder", "message": "embedding provider responds"}),
        Ok(r) => serde_json::json!({"ok": false, "provider": "embedder", "message": format!("request failed: status {}", r.status())}),
        Err(e) => serde_json::json!({"ok": false, "provider": "embedder", "message": format!("request failed: {}", e)}),
    }
}

async fn test_answer(settings: &SettingsService) -> Value {
    let base = settings.get("answer_base_url", None).unwrap_or_default();
    if base.is_empty() {
        return serde_json::json!({"ok": false, "provider": "answer", "message": "no answer base URL configured"});
    }
    let is_local = crate::common::provider_is_local(&base);
    let api_key = settings.get("answer_api_key", None).unwrap_or_default();
    if !is_local && api_key.is_empty() {
        return serde_json::json!({"ok": false, "provider": "answer", "message": "API key required for this provider"});
    }
    let model = settings.get("answer_model", None).unwrap_or_else(|| "gpt-4o-mini".to_string());
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .connect_timeout(connect_timeout())
        .build()
    {
        Ok(c) => c,
        Err(e) => return serde_json::json!({"ok": false, "provider": "answer", "message": format!("request failed: {}", e)}),
    };
    let ollama_base = settings.get("ollama_base_url", None);
    // R30.6: same request shape as the answer path (native + num_ctx for a local Ollama).
    let spec = crate::llm_chat::ChatSpec {
        base: &base,
        api_key: &api_key,
        model: &model,
        messages: serde_json::json!([{"role": "user", "content": "ping"}]),
        temperature: None,
        max_tokens: Some(1),
        timeout: std::time::Duration::from_secs(15),
        ollama_base: ollama_base.as_deref(),
    };
    match crate::llm_chat::ping(&spec, &client).await {
        Ok(()) => serde_json::json!({"ok": true, "provider": "answer", "message": format!("model '{}' responds", model)}),
        Err(message) => serde_json::json!({"ok": false, "provider": "answer", "message": message}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn env_fallback() {
        let dir = tempdir().unwrap();
        let store = crate::secrets::SecretStore::new(dir.path().join("secrets.enc"));
        let svc = SettingsService::new(store);
        // default
        assert_eq!(svc.get("ollama_base_url", None), Some("http://localhost:11434".to_string()));
    }

    #[test]
    fn llm_mode_defaults_local_and_validates() {
        // R45.2: upgrades keep today's behavior; only "agent" switches.
        let saved = std::env::var("ANCHOR_LLM_MODE").ok();
        std::env::remove_var("ANCHOR_LLM_MODE");
        let dir = tempdir().unwrap();
        let store = crate::secrets::SecretStore::new(dir.path().join("secrets.enc"));
        let svc = SettingsService::new(store);
        svc.set_data_dir(dir.path());
        let conn = Connection::open(dir.path().join("anchorcore.db")).unwrap();
        conn.execute("CREATE TABLE app_settings (key TEXT PRIMARY KEY, value TEXT)", []).unwrap();
        assert_eq!(llm_mode(&svc), LLM_MODE_LOCAL);
        svc.set("llm_mode", "Agent", &conn).unwrap();
        assert_eq!(llm_mode(&svc), LLM_MODE_AGENT);
        assert!(is_agent_mode(&svc));
        svc.set("llm_mode", "", &conn).unwrap();
        assert_eq!(llm_mode(&svc), LLM_MODE_LOCAL);
        assert!(invalid_value("llm_mode", &serde_json::json!("agent")).is_none());
        assert!(invalid_value("llm_mode", &serde_json::json!("local")).is_none());
        assert!(invalid_value("llm_mode", &serde_json::json!("")).is_none());
        assert!(invalid_value("llm_mode", &serde_json::json!("cloud")).is_some());
        assert!(invalid_value("answer_model", &serde_json::json!("anything")).is_none());
        if let Some(v) = saved {
            std::env::set_var("ANCHOR_LLM_MODE", v);
        }
    }

    #[test]
    fn empty_setting_reads_as_unset_in_all_paths() {
        // Regression (dogfood v1.0.14): the Settings UI stores `''` for
        // "Local Ollama" base URLs. Reads returned Some("") on the DB path
        // but None on the cache-hit path, so `.or_else(ollama_base_url)`
        // worked for only 3s after any other read — retrieval flapped
        // between hybrid and keyword-only. Empty must always read as unset.
        let saved_embed = std::env::var("ANCHOR_EMBED_BASE_URL").ok();
        let saved_ollama = std::env::var("ANCHOR_OLLAMA_BASE_URL").ok();
        let saved_db = std::env::var("ANCHOR_DATABASE_URL").ok();
        std::env::remove_var("ANCHOR_EMBED_BASE_URL");
        std::env::remove_var("ANCHOR_OLLAMA_BASE_URL");
        std::env::remove_var("ANCHOR_DATABASE_URL");
        let dir = tempdir().unwrap();
        let store = crate::secrets::SecretStore::new(dir.path().join("secrets.enc"));
        let svc = SettingsService::new(store);
        svc.set_data_dir(dir.path());
        let conn = Connection::open(dir.path().join("anchorcore.db")).unwrap();
        conn.execute("CREATE TABLE app_settings (key TEXT PRIMARY KEY, value TEXT)", [])
            .unwrap();
        conn.execute("INSERT INTO app_settings (key, value) VALUES ('embed_base_url', '')", [])
            .unwrap();
        // conn path, cache-miss path and cache-hit path all agree: empty == unset
        assert_eq!(svc.get("embed_base_url", Some(&conn)), None);
        assert_eq!(svc.get("embed_base_url", None), None);
        assert_eq!(svc.get("embed_base_url", None), None);
        // the consumer chain falls back to the shared Ollama base
        let base = svc
            .get("embed_base_url", None)
            .or_else(|| svc.get("ollama_base_url", None));
        assert_eq!(base, Some("http://localhost:11434".to_string()));
        if let Some(v) = saved_embed {
            std::env::set_var("ANCHOR_EMBED_BASE_URL", v);
        }
        if let Some(v) = saved_ollama {
            std::env::set_var("ANCHOR_OLLAMA_BASE_URL", v);
        }
        if let Some(v) = saved_db {
            std::env::set_var("ANCHOR_DATABASE_URL", v);
        }
    }

    #[test]
    fn storing_empty_clears_the_override_row() {
        let saved = std::env::var("ANCHOR_EMBED_BASE_URL").ok();
        std::env::remove_var("ANCHOR_EMBED_BASE_URL");
        let dir = tempdir().unwrap();
        let store = crate::secrets::SecretStore::new(dir.path().join("secrets.enc"));
        let svc = SettingsService::new(store);
        let conn = Connection::open(dir.path().join("t.db")).unwrap();
        conn.execute("CREATE TABLE app_settings (key TEXT PRIMARY KEY, value TEXT)", [])
            .unwrap();
        svc.set("embed_base_url", "http://example.test/v1", &conn)
            .unwrap();
        assert_eq!(
            svc.get("embed_base_url", Some(&conn)),
            Some("http://example.test/v1".to_string())
        );
        svc.set("embed_base_url", "", &conn).unwrap();
        let row: Option<String> = conn
            .query_row(
                "SELECT value FROM app_settings WHERE key = 'embed_base_url'",
                [],
                |r| r.get(0),
            )
            .ok();
        assert_eq!(row, None);
        assert_eq!(svc.get("embed_base_url", Some(&conn)), None);
        if let Some(v) = saved {
            std::env::set_var("ANCHOR_EMBED_BASE_URL", v);
        }
    }

    #[test]
    fn test_sentinel_shape() {
        assert!(is_test_sentinel_url("http://localhost:1"));
        assert!(is_test_sentinel_url("http://localhost:1/"));
        assert!(is_test_sentinel_url("http://127.0.0.1:1"));
        assert!(!is_test_sentinel_url("http://localhost:11434"));
        assert!(!is_test_sentinel_url(""));
    }

    #[test]
    fn test_sentinel_db_value_falls_back_to_default() {
        // A stray test-sentinel row in app_settings must not break a real
        // launch: without ANCHOR_ALLOW_TEST_URLS=1 the effective value falls
        // back to the real default. The marker is removed (and restored) so a
        // stray export in the developer's shell cannot flip this test — no
        // other test depends on the marker being set.
        let saved = std::env::var("ANCHOR_ALLOW_TEST_URLS").ok();
        std::env::remove_var("ANCHOR_ALLOW_TEST_URLS");
        let dir = tempdir().unwrap();
        let store = crate::secrets::SecretStore::new(dir.path().join("secrets.enc"));
        let svc = SettingsService::new(store);
        let conn = Connection::open(dir.path().join("t.db")).unwrap();
        conn.execute("CREATE TABLE app_settings (key TEXT PRIMARY KEY, value TEXT)", [])
            .unwrap();
        svc.set("ollama_base_url", "http://localhost:1", &conn).unwrap();
        let got = svc.get("ollama_base_url", Some(&conn));
        if let Some(v) = saved {
            std::env::set_var("ANCHOR_ALLOW_TEST_URLS", v);
        }
        assert_eq!(got, Some("http://localhost:11434".to_string()));
    }
}
