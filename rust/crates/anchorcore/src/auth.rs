//! Auth B40 — PBKDF2 + HS256 JWT, single-user local vs hosted.
//! Mirrors `backend/app/auth.py:1` + `backend/app/routers/auth.py:1`.
//! ANCHOR_AUTH_SECRET empty → auth disabled (local). Otherwise signup/login/me.

use axum::{
    extract::{Request, State},
    http::{HeaderMap, StatusCode},
    middleware::Next,
    response::Response,
    Json,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use hmac::{Hmac, Mac};
use rand::RngCore;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::Sha256;

use crate::health::AppState;

const ITER: u32 = 200_000;

// R10.8: brute-force protection — per-key (email/IP) counter, 5 fails → 60s lockout
use std::collections::HashMap;
use std::sync::{OnceLock, Mutex};
use std::time::{Duration, Instant};

static AUTH_ATTEMPTS: OnceLock<Mutex<HashMap<String, (u32, Instant, Option<Instant>)>>> = OnceLock::new();
fn attempts_map() -> &'static Mutex<HashMap<String, (u32, Instant, Option<Instant>)>> {
    AUTH_ATTEMPTS.get_or_init(|| Mutex::new(HashMap::new()))
}
const MAX_AUTH_FAILS: u32 = 5;
const AUTH_WINDOW: Duration = Duration::from_secs(60);
const AUTH_LOCKOUT: Duration = Duration::from_secs(60);

fn auth_rate_limited(key: &str) -> bool {
    let mut map = attempts_map().lock().unwrap();
    // R11.5: prune expired entries (window+lockout) to bound map
    let now = Instant::now();
    if let Some((_, last, lock)) = map.get(key).cloned() {
        if let Some(until) = lock {
            if now < until {
                return true;
            }
            // lockout expired — check if window also expired, then prune
            if now.duration_since(last) > AUTH_WINDOW + AUTH_LOCKOUT {
                map.remove(key);
                return false;
            }
            // lockout expired but still within window — not limited, but keep count
            return false;
        } else if now.duration_since(last) > AUTH_WINDOW {
            map.remove(key);
            return false;
        }
    }
    false
}
fn auth_record_failure(key: &str) {
    let mut map = attempts_map().lock().unwrap();
    let now = Instant::now();
    // prune if expired before insert
    if let Some((_, last, lock)) = map.get(key).cloned() {
        let expired = if let Some(until) = lock {
            now >= until && now.duration_since(last) > AUTH_WINDOW + AUTH_LOCKOUT
        } else {
            now.duration_since(last) > AUTH_WINDOW
        };
        if expired {
            map.remove(key);
        }
    }
    let entry = map.entry(key.to_string()).or_insert((0, now, None));
    if now.duration_since(entry.1) > AUTH_WINDOW {
        entry.0 = 0;
        entry.2 = None;
    }
    entry.0 += 1;
    entry.1 = now;
    if entry.0 >= MAX_AUTH_FAILS {
        entry.2 = Some(now + AUTH_LOCKOUT);
    }
}
fn auth_record_success(key: &str) {
    let mut map = attempts_map().lock().unwrap();
    map.remove(key);
}
fn auth_client_key(email: &str, headers: &HeaderMap) -> String {
    // R11.5: ignore client-supplied XFF/X-Real-IP by default (spoofable via `curl -H "X-Forwarded-For: ..."`).
    // Only trust proxy headers when explicitly opted in via ANCHOR_TRUSTED_PROXY=1 (hosted behind real proxy).
    if std::env::var("ANCHOR_TRUSTED_PROXY").as_deref() == Ok("1") {
        if let Some(ip) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()).or_else(|| headers.get("x-real-ip").and_then(|v| v.to_str().ok())) {
            return format!("{}:{}", email.to_lowercase(), ip.split(',').next().unwrap_or(ip).trim());
        }
    }
    email.to_lowercase()
}

pub fn auth_enabled() -> bool {
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

// --- Middleware (R7.2 route-based) ---
// NOTE (R13.6): CCN 22 is an accepted decision table — do NOT refactor (Phase 13 acceptance excludes it).
fn is_public_path(path: &str, method: &axum::http::Method) -> bool {
    // CORS preflight
    if method == axum::http::Method::OPTIONS {
        return true;
    }
    // Explicit public endpoints
    if path == "/health" || path == "/csrf" || path == "/auth/status" || path == "/auth/signup" || path == "/auth/login" {
        return true;
    }
    // Frontend assets: SPA fallback + static files — must be public so login page can load when auth is on
    if method == axum::http::Method::GET || method == axum::http::Method::HEAD {
        if path == "/" || path == "/index.html" || path.starts_with("/assets/") {
            return true;
        }
        // allow any path that doesn't look like API and has a file extension (e.g. /vite.svg, /favicon.ico)
        // API routes are under /sources, /entities, /review, /pii, /projects, /qa, /system, /settings, /auth
        let is_api = path.starts_with("/sources")
            || path.starts_with("/entities")
            || path.starts_with("/review")
            || path.starts_with("/pii")
            || path.starts_with("/projects")
            || path.starts_with("/qa")
            || path.starts_with("/system")
            || path.starts_with("/settings")
            || path.starts_with("/auth");
        if !is_api {
            // fallback SPA route (e.g. / or /login) — serve index.html, must be public
            // also static files with extension
            if !path.contains('.') {
                return true; // SPA client route
            }
            // file with extension but not API
            return true;
        }
    }
    false
}

pub async fn require_auth_middleware(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    if !auth_enabled() {
        return Ok(next.run(request).await);
    }
    let path = request.uri().path().to_string();
    let method = request.method().clone();
    if is_public_path(&path, &method) {
        return Ok(next.run(request).await);
    }
    let auth_header = request.headers().get("authorization").and_then(|v| v.to_str().ok()).unwrap_or("");
    let token = if let Some(t) = auth_header.strip_prefix("Bearer ") {
        t.trim()
    } else if let Some(t) = auth_header.strip_prefix("bearer ") {
        t.trim()
    } else {
        ""
    };
    if token.is_empty() {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let data = decode_token(token).map_err(|_| StatusCode::UNAUTHORIZED)?;
    let uid = data.get("sub").and_then(|v| v.as_i64()).unwrap_or(0);
    if uid == 0 {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let data_dir = state.data_dir.clone();
    let exists: bool = tokio::task::spawn_blocking(move || {
        let db_path = crate::db::resolve_db_path(&data_dir);
        if let Ok(conn) = crate::db::init_db(&db_path) {
            conn.query_row("SELECT 1 FROM users WHERE id=?1", [uid], |_| Ok(())).is_ok()
        } else {
            false
        }
    })
    .await
    .unwrap_or(false);
    if !exists {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(next.run(request).await)
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
    headers: HeaderMap,
    Json(payload): Json<AuthIn>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)> {
    if !auth_enabled() {
        return Err((StatusCode::NOT_FOUND, Json(json!({"detail": "Auth disabled (set ANCHOR_AUTH_SECRET)"}))));
    }
    let email_raw = payload.email.clone();
    let rate_key = auth_client_key(&email_raw, &headers);
    if auth_rate_limited(&rate_key) {
        return Err((StatusCode::TOO_MANY_REQUESTS, Json(json!({"detail": "Too many attempts, try again shortly"}))));
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
        Ok(id) => {
            auth_record_success(&rate_key);
            id
        },
        Err(e) => {
            // CONFLICT counts as failure for brute-force (enumeration)
            if e.0 == StatusCode::CONFLICT {
                auth_record_failure(&rate_key);
            }
            return Err(e);
        },
    };
    let token = create_token(user_id, &email).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"detail": e}))))?;
    Ok((StatusCode::CREATED, Json(json!({"access_token": token, "token_type": "bearer", "user": {"id": user_id, "email": email}}))))
}

pub async fn login_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<AuthIn>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    if !auth_enabled() {
        return Err((StatusCode::NOT_FOUND, Json(json!({"detail": "Auth disabled"}))));
    }
    let email_raw = payload.email.clone();
    let rate_key = auth_client_key(&email_raw, &headers);
    if auth_rate_limited(&rate_key) {
        return Err((StatusCode::TOO_MANY_REQUESTS, Json(json!({"detail": "Too many attempts, try again shortly"}))));
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
        None => {
            auth_record_failure(&rate_key);
            return Err((StatusCode::UNAUTHORIZED, Json(json!({"detail": "Invalid credentials"}))));
        },
    };
    if !verify_password(&pw, &ph) {
        auth_record_failure(&rate_key);
        return Err((StatusCode::UNAUTHORIZED, Json(json!({"detail": "Invalid credentials"}))));
    }
    auth_record_success(&rate_key);
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
    use std::sync::{Mutex, OnceLock};
    static ENV_MUTEX: OnceLock<Mutex<()>> = OnceLock::new();
    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        ENV_MUTEX.get_or_init(|| Mutex::new(())).lock().unwrap()
    }

    #[test]
    fn password_roundtrip() {
        let h = hash_password("correct horse battery staple");
        assert!(verify_password("correct horse battery staple", &h));
        assert!(!verify_password("wrong", &h));
    }

    #[test]
    fn jwt_roundtrip() {
        let _g = env_lock();
        std::env::set_var("ANCHOR_AUTH_SECRET", "test-secret-12345");
        let t = create_token(42, "a@b.c").unwrap();
        let d = decode_token(&t).unwrap();
        assert_eq!(d["sub"], 42);
        std::env::remove_var("ANCHOR_AUTH_SECRET");
    }

    #[test]
    fn auth_flag() {
        let _g = env_lock();
        std::env::remove_var("ANCHOR_AUTH_SECRET");
        assert!(!auth_enabled());
        std::env::set_var("ANCHOR_AUTH_SECRET", "x");
        assert!(auth_enabled());
        std::env::remove_var("ANCHOR_AUTH_SECRET");
    }

    #[tokio::test]
    async fn middleware_blocks_unauthenticated_post() {
        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use tower::ServiceExt;
        let _g = env_lock();
        std::env::set_var("ANCHOR_AUTH_SECRET", "test-secret-12345");
        let dir = tempfile::tempdir().unwrap();
        let store = crate::secrets::SecretStore::new(dir.path().join("secrets.enc"));
        let svc = std::sync::Arc::new(crate::settings::SettingsService::new(store));
        let jobs = crate::jobs::JobManager::new();
        let embedder = std::sync::Arc::new(crate::embedder::Embedder::new(svc.clone()));
        let state = crate::health::AppState {
            data_dir: dir.path().to_string_lossy().to_string(),
            settings: svc,
            jobs,
            csrf_token: "test-csrf".to_string(),
            embedder,
        };
        let db_path = crate::db::resolve_db_path(&state.data_dir);
        let _ = crate::db::init_db(&db_path).unwrap();
        let app = axum::Router::new()
            .route("/sources", axum::routing::get(|| async { "ok" }).post(|| async { "ok" }))
            .route("/health", axum::routing::get(|| async { "ok" }))
            .layer(axum::middleware::from_fn_with_state(state.clone(), crate::auth::require_auth_middleware))
            .with_state(state);
        // POST without token → 401
        let req = Request::builder()
            .uri("/sources")
            .method("POST")
            .body(Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        // R7.2: GET /sources is now protected when auth is on → 401
        let req2 = Request::builder()
            .uri("/sources")
            .method("GET")
            .body(Body::empty())
            .unwrap();
        let resp2 = app.clone().oneshot(req2).await.unwrap();
        assert_eq!(resp2.status(), StatusCode::UNAUTHORIZED);
        // GET /health stays public
        let req3 = Request::builder()
            .uri("/health")
            .method("GET")
            .body(Body::empty())
            .unwrap();
        let resp3 = app.clone().oneshot(req3).await.unwrap();
        assert_ne!(resp3.status(), StatusCode::UNAUTHORIZED);
        // GET / with frontend fallback should be public too
        std::env::remove_var("ANCHOR_AUTH_SECRET");
    }
}
