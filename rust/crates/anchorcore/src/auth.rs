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

// --- Middleware (R7.2 route-based, R43.8 default-deny) ---
// NOTE (R43.8): flat decision table — keep it flat (no nested API-prefix
// detection) so CCN stays ≤ 15. Replaces the R13.6 table (CCN 22), whose
// "any non-API GET is public" fallback left /items, /sections, /nodes,
// /tags (and every future route) wide open when auth was on.
fn is_public_path(path: &str, method: &axum::http::Method) -> bool {
    // CORS preflight
    if method == axum::http::Method::OPTIONS {
        return true;
    }
    // Explicit public endpoints: health, CSRF token fetch, auth bootstrap
    if path == "/health" || path == "/csrf" || path == "/auth/status" || path == "/auth/signup" || path == "/auth/login" {
        return true;
    }
    // Frontend shell + static files (GET/HEAD only) so the login page can
    // load when auth is on. The SPA is state-based (tabs via useState, no
    // path routes — App.tsx), so the only public bytes are the shell, the
    // bundle dir, and the favicon (verified against frontend/dist).
    // Default-deny: every other path requires auth when enabled.
    if method == axum::http::Method::GET || method == axum::http::Method::HEAD {
        if path == "/" || path == "/index.html" || path == "/favicon.svg" || path.starts_with("/assets/") {
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

    fn route_table_state() -> (tempfile::TempDir, crate::health::AppState) {
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
        (dir, state)
    }

    /// R43.8: default-deny — with auth on, every `build_router` route 401s
    /// without a token except the `is_public_path` allowlist. Request-level
    /// (oneshot) against the real router; CSRF middleware is a no-op when
    /// auth is on, so 401s come from the auth layer alone.
    #[tokio::test]
    async fn route_table_default_deny() {
        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use tower::ServiceExt;
        let _g = env_lock();
        std::env::set_var("ANCHOR_AUTH_SECRET", "test-secret-12345");
        let (_dir, state) = route_table_state();
        let app = crate::build_router(state);
        // Every registered route × its methods (`:id` params as `1`).
        let protected = [
            ("GET", "/sources"), ("POST", "/sources"),
            ("POST", "/sources/jira/projects"),
            ("POST", "/sources/rest/preview"),
            ("GET", "/sources/jobs"),
            ("GET", "/sources/jobs/running"),
            ("GET", "/sources/jobs/1"),
            ("POST", "/sources/jobs/1/cancel"),
            ("GET", "/sources/1"), ("PUT", "/sources/1"), ("DELETE", "/sources/1"),
            ("GET", "/sources/1/config"),
            ("POST", "/sources/1/sync"),
            ("POST", "/sources/1/reclassify"),
            ("GET", "/entities"), ("GET", "/entities/1"), ("PATCH", "/entities/1"),
            ("GET", "/entities/1/related"),
            ("POST", "/entities/1/dispute"),
            ("GET", "/entities/1/disputes"),
            ("GET", "/entities/1/conflicts"), ("POST", "/entities/1/conflicts"),
            ("DELETE", "/entities/1/conflicts/2"),
            ("GET", "/decisions"), ("GET", "/actions"),
            ("GET", "/items"), ("GET", "/items/1"), ("PATCH", "/items/1"),
            ("GET", "/items/1/outline"),
            ("GET", "/entities/1/context"),
            ("GET", "/review/low-confidence"),
            ("GET", "/review/duplicates"),
            ("POST", "/review/merge"),
            ("GET", "/glossary"), ("POST", "/glossary"), ("DELETE", "/glossary/1"),
            ("GET", "/projects"), ("POST", "/projects"),
            ("GET", "/projects/default"),
            ("GET", "/projects/1"), ("PATCH", "/projects/1"), ("DELETE", "/projects/1"),
            ("GET", "/sections"),
            ("GET", "/sections/1/chunks"),
            ("GET", "/nodes/1"),
            ("GET", "/tags"),
            ("GET", "/tags/1/chunks"),
            ("POST", "/qa"),
            ("POST", "/qa/public"),
            ("POST", "/qa/search"),
            ("GET", "/settings"), ("PUT", "/settings"),
            ("POST", "/settings/test-connection"),
            ("GET", "/system/status"),
            ("GET", "/system/onboarding"),
            ("GET", "/system/update"),
            ("GET", "/system/errors"),
            ("GET", "/system/fs"),
            ("GET", "/system/mcp"),
            ("GET", "/system/mcp/clients"),
            ("POST", "/system/mcp/connect"),
            ("POST", "/system/mcp/disconnect"),
            ("GET", "/system/logs"),
            ("GET", "/system/logs/anchorcore.log"),
            ("POST", "/system/ollama/start"),
            ("POST", "/system/ollama/pull"),
            ("GET", "/system/ollama/pulls"),
            ("GET", "/system/skills/foo"),
            ("GET", "/system/skills/install-targets"),
            ("POST", "/system/skills/foo/install"),
            ("GET", "/auth/me"),
            ("POST", "/backup/export"),
            ("POST", "/backup/import"),
            // Unknown paths: the old hole (any non-API GET was public).
            ("GET", "/login"), ("GET", "/items2"), ("GET", "/favicon.ico"),
            ("GET", "/nope.txt"), ("HEAD", "/items"), ("POST", "/items"),
        ];
        for (method, uri) in protected {
            let req = Request::builder().uri(uri).method(method).body(Body::empty()).unwrap();
            let resp = app.clone().oneshot(req).await.unwrap();
            assert_eq!(resp.status(), StatusCode::UNAUTHORIZED, "{method} {uri} must require auth");
        }
        // The allowlist stays reachable without a token.
        let public = [
            ("GET", "/health"), ("HEAD", "/health"),
            ("GET", "/csrf"),
            ("GET", "/auth/status"),
            ("POST", "/auth/signup"), ("POST", "/auth/login"),
            ("GET", "/"), ("HEAD", "/"),
            ("GET", "/index.html"),
            ("GET", "/favicon.svg"),
            ("GET", "/assets/missing-xyz.js"), // public path, 404s in fallback
            ("OPTIONS", "/sources"), ("OPTIONS", "/qa"), ("OPTIONS", "/items"),
        ];
        for (method, uri) in public {
            let req = Request::builder().uri(uri).method(method).body(Body::empty()).unwrap();
            let resp = app.clone().oneshot(req).await.unwrap();
            assert_ne!(resp.status(), StatusCode::UNAUTHORIZED, "{method} {uri} must stay public");
        }
        std::env::remove_var("ANCHOR_AUTH_SECRET");
    }

    /// R43.8: direct method×path corners the router can't reach (405s).
    #[test]
    fn public_path_method_corners() {
        use axum::http::Method;
        assert!(is_public_path("/assets/app.js", &Method::HEAD));
        assert!(is_public_path("/favicon.svg", &Method::HEAD));
        assert!(!is_public_path("/", &Method::POST));
        assert!(!is_public_path("/index.html", &Method::POST));
        // Explicit endpoints are public for any method (the router 405s
        // unregistered ones); only /auth/me among /auth/* needs a token.
        assert!(is_public_path("/health", &Method::DELETE));
        assert!(!is_public_path("/auth/me", &Method::GET));
        assert!(!is_public_path("/pii", &Method::GET));
        assert!(!is_public_path("/mcp", &Method::GET));
    }
}
