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
pub const SECRET_PREFIX: &str = "app:";
const CACHE_TTL: Duration = Duration::from_secs(3);

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
