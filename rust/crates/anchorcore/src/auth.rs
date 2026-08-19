//! Auth B40 — PBKDF2 + HS256 JWT, single-user local vs hosted.
//! Mirrors `backend/app/auth.py:1` + `backend/app/routers/auth.py:1`.
//! ANCHOR_AUTH_SECRET empty → auth disabled (local). Otherwise signup/login/me.

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    Json,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use hmac::{Hmac, Mac};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::Sha256;

use crate::health::AppState;

const ITER: u32 = 200_000;

fn auth_enabled() -> bool {
    std::env::var("ANCHOR_AUTH_SECRET")
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false)
}

fn auth_secret() -> String {
    std::env::var("ANCHOR_AUTH_SECRET").unwrap_or_default()
}

fn token_hours() -> i64 {
    std::env::var("ANCHOR_AUTH_TOKEN_HOURS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(168)
}

fn valid_email(s: &str) -> Option<String> {
    let v = s.trim().to_lowercase();
    let at = v.find('@')?;
    let dot = v[at..].find('.')?;
    if dot == 0 {
        return None;
    }
    if v.len() < 5 {
        return None;
    }
    Some(v)
}

// --- PBKDF2 ---
pub fn hash_password(password: &str) -> String {
    let mut salt_bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut salt_bytes);
    let salt = hex::encode(salt_bytes);
    let mut dk = [0u8; 32];
    pbkdf2::pbkdf2_hmac::<Sha256>(password.as_bytes(), salt.as_bytes(), ITER, &mut dk);
    format!("pbkdf2_sha256${}${}${}", ITER, salt, hex::encode(dk))
}

pub fn verify_password(password: &str, stored: &str) -> bool {
    let parts: Vec<&str> = stored.split('$').collect();
    if parts.len() != 4 {
        return false;
    }
    let it: u32 = parts[1].parse().unwrap_or(0);
    let salt = parts[2];
    let expected = parts[3];
    let mut dk = [0u8; 32];
    pbkdf2::pbkdf2_hmac::<Sha256>(password.as_bytes(), salt.as_bytes(), it, &mut dk);
    let got = hex::encode(dk);
    // constant-time compare
    if got.len() != expected.len() {
        return false;
    }
    let mut diff = 0u8;
    for (a, b) in got.bytes().zip(expected.bytes()) {
        diff |= a ^ b;
    }
    diff == 0
}

// --- JWT HS256 ---
fn b64url_encode(data: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(data)
}
fn b64url_decode(s: &str) -> Option<Vec<u8>> {
    URL_SAFE_NO_PAD.decode(s).ok()
}

pub fn create_token(user_id: i64, email: &str) -> Result<String, String> {
    if !auth_enabled() {
        return Err("auth not enabled".to_string());
    }
    let header = b64url_encode(serde_json::to_string(&json!({"alg":"HS256","typ":"JWT"})).unwrap().as_bytes());
    let exp = chrono::Utc::now().timestamp() + token_hours() * 3600;
    let payload = b64url_encode(serde_json::to_string(&json!({"sub": user_id, "email": email, "exp": exp})).unwrap().as_bytes());
    let signing_input = format!("{}.{}", header, payload);
    let mut mac = Hmac::<Sha256>::new_from_slice(auth_secret().as_bytes()).map_err(|e| e.to_string())?;
    mac.update(signing_input.as_bytes());
    let sig = b64url_encode(&mac.finalize().into_bytes());
    Ok(format!("{}.{}.{}", header, payload, sig))
}

pub fn decode_token(token: &str) -> Result<serde_json::Value, String> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err("invalid token format".to_string());
    }
    let (h, p, s) = (parts[0], parts[1], parts[2]);
    let signing_input = format!("{}.{}", h, p);
    let mut mac = Hmac::<Sha256>::new_from_slice(auth_secret().as_bytes()).map_err(|e| e.to_string())?;
    mac.update(signing_input.as_bytes());
    let expected = b64url_encode(&mac.finalize().into_bytes());
    // constant-time
    if expected.len() != s.len() {
        return Err("bad signature".to_string());
    }
    let mut diff = 0u8;
    for (a, b) in expected.bytes().zip(s.bytes()) {
        diff |= a ^ b;
    }
    if diff != 0 {
        return Err("bad signature".to_string());
    }
    let payload_bytes = b64url_decode(p).ok_or("bad payload")?;
    let data: Value = serde_json::from_slice(&payload_bytes).map_err(|e| e.to_string())?;
    let exp = data.get("exp").and_then(|v| v.as_i64()).unwrap_or(0);
    let now = chrono::Utc::now().timestamp();
    if exp < now {
        return Err("expired".to_string());
    }
    Ok(data)
}

// --- Handlers ---
pub async fn status_handler() -> Json<Value> {
    Json(json!({"enabled": auth_enabled()}))
}

#[derive(Deserialize)]
pub struct AuthIn {
    pub email: String,
    pub password: String,
}

pub async fn signup_handler(
    State(state): State<AppState>,
    Json(payload): Json<AuthIn>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)> {
    if !auth_enabled() {
        return Err((StatusCode::NOT_FOUND, Json(json!({"detail": "Auth disabled (set ANCHOR_AUTH_SECRET)"}))));
    }
    let email = match valid_email(&payload.email) {
        Some(e) => e,
        None => return Err((StatusCode::UNPROCESSABLE_ENTITY, Json(json!({"detail": "invalid email"})))),
    };
    if payload.password.len() < 8 {
        return Err((StatusCode::UNPROCESSABLE_ENTITY, Json(json!({"detail": "Password must be >=8 chars"}))));
    }
    let data_dir = state.data_dir.clone();
    let email_clone = email.clone();
    let hash = hash_password(&payload.password);
    let res = tokio::task::spawn_blocking(move || {
        let db_path = crate::db::resolve_db_path(&data_dir);
        let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| rusqlite::Connection::open(&db_path).unwrap());
        let exists: bool = conn.query_row("SELECT 1 FROM users WHERE email=?1", [&email_clone], |_| Ok(())).is_ok();
        if exists {
            return Err((StatusCode::CONFLICT, Json(json!({"detail": "Email already registered"}))));
        }
        conn.execute("INSERT INTO users (email, password_hash) VALUES (?1, ?2)", rusqlite::params![email_clone, hash]).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"detail": e.to_string()}))))?;
        let id = conn.last_insert_rowid();
        Ok::<i64, (StatusCode, Json<Value>)>(id)
    })
    .await
    .unwrap();
    let user_id = match res {
        Ok(id) => id,
        Err(e) => return Err(e),
    };
    let token = create_token(user_id, &email).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"detail": e}))))?;
    Ok((StatusCode::CREATED, Json(json!({"access_token": token, "token_type": "bearer", "user": {"id": user_id, "email": email}}))))
}

pub async fn login_handler(
    State(state): State<AppState>,
    Json(payload): Json<AuthIn>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    if !auth_enabled() {
        return Err((StatusCode::NOT_FOUND, Json(json!({"detail": "Auth disabled"}))));
    }
    let email = match valid_email(&payload.email) {
        Some(e) => e,
        None => return Err((StatusCode::UNPROCESSABLE_ENTITY, Json(json!({"detail": "invalid email"})))),
    };
    let data_dir = state.data_dir.clone();
    let email_clone = email.clone();
    let pw = payload.password.clone();
    let row: Option<(i64, String)> = tokio::task::spawn_blocking(move || {
        let db_path = crate::db::resolve_db_path(&data_dir);
        let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| rusqlite::Connection::open(&db_path).unwrap());
        let res: Result<(i64, String), _> = conn.query_row("SELECT id, password_hash FROM users WHERE email=?1", [&email_clone], |r| Ok((r.get(0)?, r.get(1)?)));
        res.ok()
    })
    .await
    .unwrap();
    let (uid, ph) = match row {
        Some(v) => v,
        None => return Err((StatusCode::UNAUTHORIZED, Json(json!({"detail": "Invalid credentials"})))),
    };
    if !verify_password(&pw, &ph) {
        return Err((StatusCode::UNAUTHORIZED, Json(json!({"detail": "Invalid credentials"}))));
    }
    let token = create_token(uid, &email).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"detail": e}))))?;
    Ok(Json(json!({"access_token": token, "token_type": "bearer", "user": {"id": uid, "email": email}})))
}

pub async fn me_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    if !auth_enabled() {
        return Err((StatusCode::NOT_FOUND, Json(json!({"detail": "Auth disabled"}))));
    }
    let auth = headers.get("authorization").and_then(|v| v.to_str().ok()).unwrap_or("");
    let token = if let Some(stripped) = auth.strip_prefix("Bearer ") {
        stripped.trim()
    } else if let Some(stripped) = auth.strip_prefix("bearer ") {
        stripped.trim()
    } else {
        ""
    };
    if token.is_empty() {
        return Err((StatusCode::UNAUTHORIZED, Json(json!({"detail": "Not authenticated"}))));
    }
    let data = decode_token(token).map_err(|e| (StatusCode::UNAUTHORIZED, Json(json!({"detail": e}))))?;
    let uid = data.get("sub").and_then(|v| v.as_i64()).unwrap_or(0);
    let email = data.get("email").and_then(|v| v.as_str()).unwrap_or("").to_string();
    // verify user exists
    let data_dir = state.data_dir.clone();
    let exists: bool = tokio::task::spawn_blocking(move || {
        let db_path = crate::db::resolve_db_path(&data_dir);
        let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| rusqlite::Connection::open(&db_path).unwrap());
        conn.query_row("SELECT 1 FROM users WHERE id=?1", [uid], |_| Ok(())).is_ok()
    })
    .await
    .unwrap();
    if !exists {
        return Err((StatusCode::UNAUTHORIZED, Json(json!({"detail": "User not found"}))));
    }
    Ok(Json(json!({"id": uid, "email": email})))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_roundtrip() {
        let h = hash_password("correct horse battery staple");
        assert!(verify_password("correct horse battery staple", &h));
        assert!(!verify_password("wrong", &h));
    }

    #[test]
    fn jwt_roundtrip() {
        std::env::set_var("ANCHOR_AUTH_SECRET", "test-secret-12345");
        let t = create_token(42, "a@b.c").unwrap();
        let d = decode_token(&t).unwrap();
        assert_eq!(d["sub"], 42);
        std::env::remove_var("ANCHOR_AUTH_SECRET");
    }
}
