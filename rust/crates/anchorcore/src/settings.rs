use crate::secrets::SecretStore;
use rusqlite::Connection;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

pub const SETTING_KEYS: &[&str] = &[
    "ollama_base_url",
    "classifier_model",
    "embed_model",
    "classifier_timeout",
    "classifier_base_url",
    "embed_base_url",
    "answer_model",
    "answer_base_url",
    "answer_timeout",
    "answer_reasoning_effort",
];
pub const SECRET_KEYS: &[&str] = &["answer_api_key", "classifier_api_key", "embed_api_key"];
pub const ALL_KEYS: &[&str] = &[
    "ollama_base_url",
    "classifier_model",
    "embed_model",
    "classifier_timeout",
    "classifier_base_url",
    "embed_base_url",
    "answer_model",
    "answer_base_url",
    "answer_timeout",
    "answer_reasoning_effort",
    "answer_api_key",
    "classifier_api_key",
    "embed_api_key",
];
pub const SECRET_PREFIX: &str = "app:";
const CACHE_TTL: Duration = Duration::from_secs(3);

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
        "classifier_model" => Some("llama3.2:3b".to_string()),
        "embed_model" => Some("nomic-embed-text".to_string()),
        "answer_model" => Some("gpt-4o-mini".to_string()),
        "answer_base_url" => Some("https://api.openai.com/v1".to_string()),
        "classifier_base_url" => None,
        "embed_base_url" => None,
        "answer_reasoning_effort" => Some("none".to_string()),
        _ => None,
    }
}

pub struct SettingsService {
    secrets: SecretStore,
    cache: Mutex<HashMap<String, (String, Instant)>>,
}

impl SettingsService {
    pub fn new(secrets: SecretStore) -> Self {
        Self {
            secrets,
            cache: Mutex::new(HashMap::new()),
        }
    }

    pub fn get(&self, key: &str, conn: Option<&Connection>) -> Option<String> {
        if !SETTING_KEYS.contains(&key) && !SECRET_KEYS.contains(&key) {
            return env_value(key);
        }
        if SECRET_KEYS.contains(&key) {
            let skey = format!("{}{}", SECRET_PREFIX, key);
            if let Some(v) = self.secrets.get(&skey) {
                return Some(v);
            }
            return env_value(key);
        }
        // non-secret: check DB first if conn provided, else cache
        if let Some(c) = conn {
            if let Some(v) = db_value(c, key) {
                return Some(v);
            }
            return env_value(key);
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
            let cache = self.cache.lock().unwrap();
            if let Some((v, exp)) = cache.get(key) {
                if Instant::now() < *exp {
                    return if v.is_empty() { None } else { Some(v.clone()) };
                }
            }
        }
        // need DB read; open a short-lived connection to data dir
        // For simplicity, we try to open the default DB file (respect ANCHOR_DATABASE_URL)
        let data_dir = std::env::var("ANCHOR_DATA_DIR").unwrap_or_else(|_| "data".to_string());
        let db_path = crate::db::resolve_db_path(&data_dir);
        let conn = Connection::open(&db_path).ok();
        let db_val = conn.as_ref().and_then(|c| db_value(c, key));
        let resolved = db_val.or_else(|| env_value(key));
        let mut cache = self.cache.lock().unwrap();
        cache.insert(
            key.to_string(),
            (resolved.clone().unwrap_or_default(), Instant::now() + CACHE_TTL),
        );
        resolved
    }

    fn invalidate(&self, key: &str) {
        self.cache.lock().unwrap().remove(key);
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
        "classifier" => Ok(Json(test_classifier(&state.settings).await)),
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
    // Conftest sets ANCHOR_HTTP_CONNECT_TIMEOUT=0.2 to keep suite fast (Windows quirk)
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
    let classifier = settings
        .get("classifier_model", None)
        .unwrap_or_else(|| "llama3.2:3b".to_string())
        .split(':')
        .next()
        .unwrap_or("llama3.2")
        .to_string();
    let embed = settings
        .get("embed_model", None)
        .unwrap_or_else(|| "nomic-embed-text".to_string())
        .split(':')
        .next()
        .unwrap_or("nomic-embed-text")
        .to_string();
    let missing: Vec<String> = [classifier.clone(), embed.clone()]
        .into_iter()
        .filter(|m| !models.contains(m))
        .collect();
    if !missing.is_empty() {
        return serde_json::json!({"ok": false, "provider": "ollama", "message": format!("reachable; models missing: {} (run: ollama pull {})", missing.join(", "), missing.join(" && ollama pull "))});
    }
    serde_json::json!({"ok": true, "provider": "ollama", "message": "Ollama up, models present"})
}

async fn test_classifier(settings: &SettingsService) -> Value {
    let base = settings
        .get("classifier_base_url", None)
        .or_else(|| settings.get("ollama_base_url", None))
        .unwrap_or_else(|| "http://localhost:11434".to_string());
    let is_local = base.starts_with("http://localhost") || base.starts_with("http://127.0.0.1");
    let api_key = settings.get("classifier_api_key", None).unwrap_or_default();
    if !is_local && api_key.is_empty() {
        return serde_json::json!({"ok": false, "provider": "classifier", "message": "API key required for a cloud classifier provider"});
    }
    let model = settings.get("classifier_model", None).unwrap_or_else(|| "llama3.2:3b".to_string());
    let url = format!("{}/v1/chat/completions", base.trim_end_matches('/'));
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .connect_timeout(connect_timeout())
        .build()
    {
        Ok(c) => c,
        Err(e) => return serde_json::json!({"ok": false, "provider": "classifier", "message": format!("request failed: {}", e)}),
    };
    let mut req = client.post(&url).json(&serde_json::json!({
        "model": model,
        "messages": [{"role":"user","content":"ping"}],
        "max_tokens": 1
    }));
    if !api_key.is_empty() {
        req = req.header("Authorization", format!("Bearer {}", api_key));
    }
    match req.send().await {
        Ok(r) if r.status().is_success() => serde_json::json!({"ok": true, "provider": "classifier", "message": "classifier provider responds"}),
        Ok(r) => serde_json::json!({"ok": false, "provider": "classifier", "message": format!("request failed: status {}", r.status())}),
        Err(e) => serde_json::json!({"ok": false, "provider": "classifier", "message": format!("request failed: {}", e)}),
    }
}

async fn test_embedder(settings: &SettingsService) -> Value {
    let base = settings
        .get("embed_base_url", None)
        .or_else(|| settings.get("ollama_base_url", None))
        .unwrap_or_else(|| "http://localhost:11434".to_string());
    let is_local = base.starts_with("http://localhost") || base.starts_with("http://127.0.0.1");
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
    let is_local = base.starts_with("http://localhost") || base.starts_with("http://127.0.0.1");
    let api_key = settings.get("answer_api_key", None).unwrap_or_default();
    if !is_local && api_key.is_empty() {
        return serde_json::json!({"ok": false, "provider": "answer", "message": "API key required for this provider"});
    }
    let model = settings.get("answer_model", None).unwrap_or_else(|| "gpt-4o-mini".to_string());
    // Python posts to {base}/chat/completions (base already includes /v1)
    let url = format!("{}/chat/completions", base.trim_end_matches('/'));
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .connect_timeout(connect_timeout())
        .build()
    {
        Ok(c) => c,
        Err(e) => return serde_json::json!({"ok": false, "provider": "answer", "message": format!("request failed: {}", e)}),
    };
    let mut req = client.post(&url).json(&serde_json::json!({
        "model": model,
        "messages": [{"role":"user","content":"ping"}],
        "max_tokens": 1
    }));
    if !api_key.is_empty() {
        req = req.header("Authorization", format!("Bearer {}", api_key));
    }
    match req.send().await {
        Ok(r) if r.status().is_success() => serde_json::json!({"ok": true, "provider": "answer", "message": format!("model '{}' responds", model)}),
        Ok(r) => serde_json::json!({"ok": false, "provider": "answer", "message": format!("request failed: status {}", r.status())}),
        Err(e) => serde_json::json!({"ok": false, "provider": "answer", "message": format!("request failed: {}", e)}),
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
}
