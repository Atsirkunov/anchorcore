//! R7.1: Host/Origin validation + CORS allowlist + CSRF token
//! When auth is disabled (local mode), the server is 127.0.0.1:<port> with no auth.
//! Without checks, any website can `fetch('http://127.0.0.1:8123/entities')` and exfiltrate.
//! This module implements:
//!  - allowed Host/Origin check (allowlist: localhost, 127.0.0.1, ::1 + ANCHOR_CORS_ORIGINS)
//!  - narrow CORS (not permissive)
//!  - per-session CSRF token for mutating requests when auth disabled

use axum::{
    extract::{Request, State},
    http::{HeaderValue, StatusCode},
    middleware::Next,
    response::Response,
    Json,
};
use serde_json::json;
use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

use crate::health::AppState;

static CACHED_HOSTS: OnceLock<Mutex<(Option<String>, HashSet<String>)>> = OnceLock::new();
static CACHED_ORIGINS: OnceLock<Mutex<(Option<String>, Vec<String>)>> = OnceLock::new();

// ----- CSRF -----
pub fn generate_csrf_token() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

// ----- Allowlist (R11.8: cached OnceLock) -----
fn allowed_hosts_set() -> HashSet<String> {
    let env_val = std::env::var("ANCHOR_CORS_ORIGINS").ok();
    let cache = CACHED_HOSTS.get_or_init(|| Mutex::new((None, HashSet::new())));
    let mut guard = cache.lock().unwrap();
    if guard.0 == env_val && !guard.1.is_empty() {
        return guard.1.clone();
    }
    let mut set = HashSet::new();
    for h in ["localhost", "127.0.0.1", "::1", "[::1]"] {
        set.insert(h.to_string());
    }
    if let Some(v) = &env_val {
        for part in v.split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            let host = if part.contains("://") {
                let after_scheme = part.split("://").nth(1).unwrap_or(part);
                let host_port = after_scheme.split('/').next().unwrap_or(after_scheme);
                if host_port.starts_with('[') {
                    if let Some(end) = host_port.find(']') {
                        &host_port[..end + 1]
                    } else {
                        host_port
                    }
                } else {
                    host_port.split(':').next().unwrap_or(host_port)
                }
            } else if part.starts_with('[') {
                if let Some(end) = part.find(']') {
                    &part[..end + 1]
                } else {
                    part
                }
            } else {
                part.split(':').next().unwrap_or(part)
            };
            set.insert(host.to_lowercase());
            if host == "[::1]" {
                set.insert("::1".to_string());
            }
        }
    }
    guard.0 = env_val;
    guard.1 = set.clone();
    set
}

fn allowed_origin_prefixes() -> Vec<String> {
    let env_val = std::env::var("ANCHOR_CORS_ORIGINS").ok();
    let cache = CACHED_ORIGINS.get_or_init(|| Mutex::new((None, Vec::new())));
    let mut guard = cache.lock().unwrap();
    if guard.0 == env_val && !guard.1.is_empty() {
        return guard.1.clone();
    }
    let mut prefixes = vec![
        "http://localhost".to_string(),
        "http://127.0.0.1".to_string(),
        "http://[::1]".to_string(),
    ];
    if let Some(v) = &env_val {
        for part in v.split(',') {
            let p = part.trim().trim_end_matches('/');
            if !p.is_empty() {
                prefixes.push(p.to_string());
            }
        }
    }
    guard.0 = env_val.clone();
    guard.1 = prefixes.clone();
    prefixes
}

pub fn is_host_allowed(host: &str) -> bool {
    if host.is_empty() {
        return true; // allow missing Host for non-browser clients (curl, tests)
    }
    let host = host.trim();
    // extract host without port, handle IPv6
    let host_only = if host.starts_with('[') {
        // [::1]:port or [::1]
        if let Some(end) = host.find(']') {
            &host[..end + 1]
        } else {
            host
        }
    } else {
        host.split(':').next().unwrap_or(host)
    };
    let host_only = host_only.to_lowercase();
    let allowed = allowed_hosts_set();
    // also allow case where host_only includes brackets vs not
    if allowed.contains(&host_only) {
        return true;
    }
    // normalize ::1 variants
    if (host_only == "::1" || host_only == "[::1]") && (allowed.contains("::1") || allowed.contains("[::1]")) {
        return true;
    }
    false
}

pub fn is_origin_allowed(origin: &str) -> bool {
    if origin.is_empty() {
        return true;
    }
    let origin = origin.trim().trim_end_matches('/');
    let prefixes = allowed_origin_prefixes();
    for prefix in prefixes {
        if origin == prefix {
            return true;
        }
        if origin.starts_with(&prefix) {
            let rest = &origin[prefix.len()..];
            if rest.is_empty() || rest.starts_with(':') || rest.starts_with('/') {
                return true;
            }
        }
    }
    false
}

// ----- Middleware: Host/Origin validation -----
pub async fn host_origin_middleware(
    State(_state): State<AppState>,
    request: Request,
    next: Next,
) -> Result<Response, (StatusCode, Json<serde_json::Value>)> {
    // Check Host header
    if let Some(host_val) = request.headers().get("host").and_then(|v| v.to_str().ok()) {
        if !is_host_allowed(host_val) {
            return Err((
                StatusCode::FORBIDDEN,
                Json(json!({"detail": "Forbidden: invalid Host"})),
            ));
        }
    }
    // Check Origin header if present
    if let Some(origin_val) = request.headers().get("origin").and_then(|v| v.to_str().ok()) {
        if !is_origin_allowed(origin_val) {
            return Err((
                StatusCode::FORBIDDEN,
                Json(json!({"detail": "Forbidden: invalid Origin"})),
            ));
        }
    }
    // Also check Referer as fallback for older browsers? optional
    Ok(next.run(request).await)
}

// ----- Middleware: CSRF -----
pub async fn csrf_middleware(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Result<Response, (StatusCode, Json<serde_json::Value>)> {
    // Only when auth is disabled (local mode)
    if crate::auth::auth_enabled() {
        return Ok(next.run(request).await);
    }
    let method = request.method().clone();
    let is_mutating = method == axum::http::Method::POST
        || method == axum::http::Method::PUT
        || method == axum::http::Method::PATCH
        || method == axum::http::Method::DELETE;

    if !is_mutating {
        return Ok(next.run(request).await);
    }

    let path = request.uri().path().to_string();
    // Exempt health, csrf endpoint, and auth status/login/signup (though those are not mutating except signup/login which should still require CSRF? but they are public auth endpoints, allow without CSRF to enable signup)
    if path == "/health" || path == "/csrf" || path == "/auth/signup" || path == "/auth/login" || path == "/auth/status" {
        return Ok(next.run(request).await);
    }

    let token = &state.csrf_token;
    let header_val = request
        .headers()
        .get("x-csrf-token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if std::env::var("ANCHOR_CSRF_DISABLE").as_deref() == Ok("1") {
        return Ok(next.run(request).await);
    }

    if header_val == token {
        return Ok(next.run(request).await);
    }
    Err((
        StatusCode::FORBIDDEN,
        Json(json!({"detail": "Forbidden: missing or invalid CSRF token (X-CSRF-Token)"})),
    ))
}

// ----- CORS builder -----
pub fn cors_layer() -> tower_http::cors::CorsLayer {
    use tower_http::cors::{AllowHeaders, AllowMethods, AllowOrigin};

    let allow_origin = AllowOrigin::predicate(|origin: &HeaderValue, _| {
        if let Ok(s) = origin.to_str() {
            is_origin_allowed(s)
        } else {
            false
        }
    });

    tower_http::cors::CorsLayer::new()
        .allow_origin(allow_origin)
        .allow_methods(AllowMethods::mirror_request())
        .allow_headers(AllowHeaders::mirror_request())
        .allow_credentials(true)
}

// ----- Handler for CSRF token -----
pub async fn csrf_handler(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(json!({"csrf_token": state.csrf_token}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_allowed() {
        assert!(is_host_allowed("localhost:8123"));
        assert!(is_host_allowed("127.0.0.1:8123"));
        assert!(is_host_allowed("localhost"));
        assert!(is_host_allowed("127.0.0.1"));
        assert!(is_host_allowed("[::1]:8123"));
        assert!(!is_host_allowed("evil.com"));
        assert!(!is_host_allowed("localhost.evil.com"));
        assert!(!is_host_allowed("127.0.0.1.evil.com"));
    }

    #[test]
    fn origin_allowed() {
        assert!(is_origin_allowed("http://localhost:8123"));
        assert!(is_origin_allowed("http://127.0.0.1:8123"));
        assert!(is_origin_allowed("http://localhost"));
        assert!(is_origin_allowed("http://[::1]:8123"));
        assert!(!is_origin_allowed("https://evil.com"));
        assert!(!is_origin_allowed("http://evil.com"));
        assert!(!is_origin_allowed("http://localhost.evil.com"));
        assert!(is_origin_allowed("")); // missing
    }

    #[test]
    fn origin_allowlist_env() {
        std::env::set_var("ANCHOR_CORS_ORIGINS", "https://app.example.com, http://localhost:3000");
        assert!(is_origin_allowed("https://app.example.com"));
        assert!(is_origin_allowed("https://app.example.com:443"));
        assert!(is_host_allowed("app.example.com"));
        std::env::remove_var("ANCHOR_CORS_ORIGINS");
    }
}
