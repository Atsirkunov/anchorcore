use crate::fernet;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub const SECRET_SOURCE_FIELDS: &[&str] = &["token", "api_key", "password", "client_secret"];
const SERVICE: &str = "AnchorCore";

pub struct SecretStore {
    fallback_path: PathBuf,
    use_keyring: bool,
    fernet: Option<fernet::Fernet>,
}

impl SecretStore {
    pub fn new(fallback_path: PathBuf) -> Self {
        let use_keyring = std::env::var("ANCHOR_SECRETS_NO_KEYRING").as_deref() != Ok("1");
        let fernet = Self::load_fernet(&fallback_path);
        Self {
            fallback_path,
            use_keyring,
            fernet,
        }
    }

    fn load_fernet(fallback_path: &Path) -> Option<fernet::Fernet> {
        let key_path = fallback_path.with_extension("key");
        // fernet key is 32 url-safe base64-encoded bytes (44 chars)
        if key_path.exists() {
            if let Ok(s) = std::fs::read_to_string(&key_path) {
                let s = s.trim();
                if let Some(f) = fernet::Fernet::new(s) {
                    return Some(f);
                }
            }
        }
        let key = fernet::Fernet::generate_key();
        let _ = std::fs::create_dir_all(key_path.parent().unwrap());
        let _ = std::fs::write(&key_path, &key);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600));
        }
        fernet::Fernet::new(&key)
    }

    fn fallback_file(&self, key: &str) -> PathBuf {
        let mut hasher = Sha256::new();
        hasher.update(key.as_bytes());
        let digest = hex::encode(hasher.finalize());
        let name = format!(
            "{}.{}",
            self.fallback_path.file_name().unwrap().to_string_lossy(),
            &digest
        );
        self.fallback_path.with_file_name(name)
    }

    pub fn set(&self, key: &str, value: &str) {
        if self.use_keyring {
            if let Ok(entry) = keyring::Entry::new(SERVICE, key) {
                if entry.set_password(value).is_ok() {
                    return;
                } else {
                    tracing::warn!("keyring unavailable for set({}); using fallback", key);
                }
            }
        }
        if let Some(f) = &self.fernet {
            let enc = f.encrypt(value.as_bytes());
            let path = self.fallback_file(key);
            let _ = std::fs::create_dir_all(path.parent().unwrap());
            let _ = std::fs::write(&path, enc);
        } else {
            let path = self.fallback_file(key);
            let _ = std::fs::create_dir_all(path.parent().unwrap());
            let _ = std::fs::write(&path, value.as_bytes());
        }
    }

    pub fn get(&self, key: &str) -> Option<String> {
        if self.use_keyring {
            if let Ok(entry) = keyring::Entry::new(SERVICE, key) {
                if let Ok(v) = entry.get_password() {
                    return Some(v);
                }
            }
        }
        let path = self.fallback_file(key);
        if !path.exists() {
            return None;
        }
        let data = std::fs::read(&path).ok()?;
        if let Some(f) = &self.fernet {
            if let Ok(dec) = f.decrypt(&String::from_utf8_lossy(&data)) {
                return String::from_utf8(dec).ok();
            }
            return None;
        }
        String::from_utf8(data).ok()
    }

    pub fn delete(&self, key: &str) {
        if self.use_keyring {
            if let Ok(entry) = keyring::Entry::new(SERVICE, key) {
                let _ = entry.delete_password();
            }
        }
        let path = self.fallback_file(key);
        let _ = std::fs::remove_file(path);
    }
}

#[allow(dead_code)]
pub fn source_secret_key(source_id: i64, field: &str) -> String {
    format!("source:{}:{}", source_id, field)
}

/// Mirrors `backend/app/secrets.py:105` store_source_config
#[allow(dead_code)]
pub fn store_source_config(
    conn: &rusqlite::Connection,
    source_id: i64,
    mut config: serde_json::Value,
    secrets: &SecretStore,
) -> serde_json::Value {
    let obj = config.as_object_mut().expect("config must be object");
    for &field in SECRET_SOURCE_FIELDS {
        let value = obj.remove(field);
        match value {
            None => continue, // absent -> keep stored
            Some(v) if v == serde_json::Value::String("***set***".to_string()) => continue,
            Some(serde_json::Value::String(s)) if s.is_empty() => {
                secrets.delete(&source_secret_key(source_id, field));
            }
            Some(serde_json::Value::String(s)) => {
                secrets.set(&source_secret_key(source_id, field), &s);
            }
            Some(other) => {
                let s = other.as_str().unwrap_or("").to_string();
                if s.is_empty() {
                    secrets.delete(&source_secret_key(source_id, field));
                } else {
                    secrets.set(&source_secret_key(source_id, field), &s);
                }
            }
        }
    }
    // persist safe config (without secrets) as JSON in sources.config
    let safe_str = serde_json::to_string(&config).unwrap_or_else(|_| "{}".to_string());
    let _ = conn.execute(
        "UPDATE sources SET config = ?1 WHERE id = ?2",
        rusqlite::params![safe_str, source_id],
    );
    config
}

#[allow(dead_code)]
pub fn resolve_source_config(
    conn: &rusqlite::Connection,
    source_id: i64,
    secrets: &SecretStore,
) -> serde_json::Value {
    let config_str: String = conn
        .query_row(
            "SELECT config FROM sources WHERE id = ?1",
            rusqlite::params![source_id],
            |r| r.get(0),
        )
        .unwrap_or_else(|_| "{}".to_string());
    let mut config: serde_json::Value =
        serde_json::from_str(&config_str).unwrap_or_else(|_| serde_json::json!({}));
    let obj = config.as_object_mut().unwrap();
    for &field in SECRET_SOURCE_FIELDS {
        if let Some(v) = secrets.get(&source_secret_key(source_id, field)) {
            obj.insert(field.to_string(), serde_json::Value::String(v));
        }
    }
    config
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn placeholder_keeps_secret() {
        let dir = tempdir().unwrap();
        let _store = SecretStore::new(dir.path().join("secrets.enc"));
        // set via env force fallback
        std::env::set_var("ANCHOR_SECRETS_NO_KEYRING", "1");
        let store2 = SecretStore::new(dir.path().join("secrets.enc"));
        store2.set("k1", "tok123");
        assert_eq!(store2.get("k1"), Some("tok123".to_string()));
        store2.delete("k1");
        assert_eq!(store2.get("k1"), None);
        std::env::remove_var("ANCHOR_SECRETS_NO_KEYRING");
    }
}
