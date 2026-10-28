//! R41.7: visible startup. The server binds the port (tray + browser) before
//! the database is ready; `db::init_db_tracked` reports its phases here and
//! the gate below keeps every API route at 503 until boot finishes.
//!
//! One process-wide status. The default is `Ready`, so code that never calls
//! [`begin`] (unit tests, the CLI, `anchorcore mcp`) is never gated, and
//! reports made outside a boot are ignored.

use axum::{
    extract::Request,
    http::{header, Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};

const READY: u8 = 0;
const STARTING: u8 = 1;
const FAILED: u8 = 2;

/// Body `detail` of every gated API route while the database upgrades.
pub const UPGRADING_DETAIL: &str = "AnchorCore is upgrading its database";
pub const FAILED_DETAIL: &str = "AnchorCore could not upgrade its database — open AnchorCore to see what happened";

static STATE: AtomicU8 = AtomicU8::new(READY);
static STATUS: OnceLock<Mutex<Status>> = OnceLock::new();
static LISTENER: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();

#[derive(Clone, Debug, Default)]
struct Status {
    phase: &'static str,
    step: u32,
    of: u32,
    detail: String,
    started_at: String,
    /// Upgrading an existing library (vs. creating a fresh one / a quick boot).
    upgrading: bool,
    progress: Option<(u64, u64, &'static str)>,
    error: Option<String>,
    data_state: Option<String>,
    log_file: String,
}

fn status() -> MutexGuard<'static, Status> {
    STATUS.get_or_init(|| Mutex::new(Status::default())).lock().unwrap_or_else(|e| e.into_inner())
}

fn starting() -> bool {
    STATE.load(Ordering::SeqCst) == STARTING
}

fn changed() {
    if let Some(f) = LISTENER.get() {
        f();
    }
}

/// Called (once) by the tray so its tooltip follows the status.
pub fn on_change(f: Box<dyn Fn() + Send + Sync>) {
    let _ = LISTENER.set(f);
}

/// Boot begins: every API route answers 503 until [`ready`] or [`fail`].
pub fn begin(log_file: &std::path::Path) {
    {
        let mut s = status();
        *s = Status {
            phase: "preparing",
            step: 0,
            of: 0,
            detail: "Opening your library".to_string(),
            started_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            log_file: redact_home(&log_file.to_string_lossy()),
            ..Status::default()
        };
    }
    STATE.store(STARTING, Ordering::SeqCst);
    changed();
}

pub fn is_ready() -> bool {
    STATE.load(Ordering::SeqCst) == READY
}

pub fn is_failed() -> bool {
    STATE.load(Ordering::SeqCst) == FAILED
}

/// The boot will run `of` steps; `upgrading` = an existing library changes.
pub fn plan(of: u32, upgrading: bool) {
    if !starting() {
        return;
    }
    let mut s = status();
    s.of = of;
    s.upgrading = upgrading;
}

/// Step `step` of the plan begins (`phase` is machine-readable, `detail` is
/// what the person reads).
pub fn step(step: u32, phase: &'static str, detail: &str) {
    if !starting() {
        return;
    }
    {
        let mut s = status();
        s.step = step;
        s.phase = phase;
        s.detail = detail.to_string();
        s.progress = None;
    }
    changed();
}

/// New wording for the current step (same step number).
pub fn step_detail(detail: &str) {
    if !starting() {
        return;
    }
    status().detail = detail.to_string();
    changed();
}

/// Progress inside the current step (`unit`: "bytes" or "items").
pub fn progress(done: u64, total: u64, unit: &'static str) {
    if !starting() {
        return;
    }
    status().progress = Some((done, total, unit));
}

pub fn ready() {
    STATE.store(READY, Ordering::SeqCst);
    changed();
}

/// Boot failed: keep serving, show `error` (redacted) and what state the data
/// is in. The process never exits silently on a failed upgrade.
pub fn fail(error: &str, data_state: &str) {
    {
        let mut s = status();
        s.error = Some(redact_home(error));
        s.data_state = Some(data_state.to_string());
        s.progress = None;
    }
    STATE.store(FAILED, Ordering::SeqCst);
    changed();
}

/// Home directory → `~`, so the error and log path never carry a user name.
fn redact_home(s: &str) -> String {
    let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).unwrap_or_default();
    if home.len() > 1 {
        s.replace(&home, "~")
    } else {
        s.to_string()
    }
}

/// `/health` body while not ready (`status` = `starting` | `failed`).
pub fn snapshot() -> Value {
    let s = status().clone();
    let failed = is_failed();
    let mut v = json!({
        "status": if failed { "failed" } else { "starting" },
        "phase": if failed { "failed" } else { s.phase },
        "step": s.step,
        "of": s.of,
        "detail": s.detail,
        "started_at": s.started_at,
        "upgrading": s.upgrading,
        "version": env!("CARGO_PKG_VERSION"),
        "progress": s.progress.map(|(done, total, unit)| json!({"done": done, "total": total, "unit": unit})),
    });
    if failed {
        let error = s.error.clone().unwrap_or_default();
        v["error"] = json!(error);
        v["data_state"] = json!(s.data_state.clone().unwrap_or_default());
        v["log_file"] = json!(s.log_file);
        v["support"] = json!(support_text(&s, &error));
    }
    v
}

/// What the person pastes to support: versions + step + redacted error.
fn support_text(s: &Status, error: &str) -> String {
    format!(
        "AnchorCore {} on {} ({}): database upgrade failed at step {} of {} ({}).\nError: {}\nData: {}",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        s.step,
        s.of,
        s.detail,
        error,
        s.data_state.as_deref().unwrap_or("")
    )
}

/// Tray tooltip / status line for the current state.
pub fn tray_label() -> String {
    match STATE.load(Ordering::SeqCst) {
        READY => "AnchorCore".to_string(),
        FAILED => "AnchorCore — database upgrade failed (open AnchorCore for details)".to_string(),
        _ => {
            let s = status();
            let what = if s.upgrading { "Upgrading database…" } else { "Starting…" };
            if s.of > 1 && s.step > 0 {
                format!("AnchorCore — {what} (step {} of {})", s.step, s.of)
            } else {
                format!("AnchorCore — {what}")
            }
        }
    }
}

/// One line for agents (MCP): "upgrading (step N of M), try again shortly".
pub fn agent_message(body: &Value) -> String {
    if body.get("status").and_then(Value::as_str) == Some("failed") {
        let err = body.get("error").and_then(Value::as_str).unwrap_or("unknown error");
        return format!("AnchorCore could not upgrade its database ({err}). Ask the user to open AnchorCore: it shows what happened and what to send to support.");
    }
    let step = body.get("step").and_then(Value::as_u64).unwrap_or(0);
    let of = body.get("of").and_then(Value::as_u64).unwrap_or(0);
    let detail = body.get("detail").and_then(Value::as_str).unwrap_or("");
    let at = if of > 0 && step > 0 { format!(" (step {step} of {of}: {detail})") } else { String::new() };
    format!("AnchorCore is upgrading its database{at}, try again shortly. This is not an error and the app is running; large libraries can take several minutes.")
}

/// The SPA shell stays served while starting so it can show the upgrade
/// screen (same public bytes as the auth gate: shell, bundle dir, favicon).
fn is_shell_path(path: &str, method: &Method) -> bool {
    if method != Method::GET && method != Method::HEAD {
        return false;
    }
    path == "/" || path == "/index.html" || path == "/favicon.svg" || path.starts_with("/assets/")
}

/// Router layer (outside CSRF/auth, which may touch the DB): while not
/// ready, `/health` answers the status, the SPA shell passes, every other
/// route is 503.
pub async fn gate(request: Request, next: Next) -> Response {
    if is_ready() {
        return next.run(request).await;
    }
    let path = request.uri().path();
    if path == "/health" {
        return unavailable(snapshot());
    }
    if is_shell_path(path, request.method()) {
        return next.run(request).await;
    }
    let snap = snapshot();
    let detail = if is_failed() { FAILED_DETAIL } else { UPGRADING_DETAIL };
    unavailable(json!({
        "detail": detail,
        "status": snap["status"],
        "phase": snap["phase"],
        "step": snap["step"],
        "of": snap["of"],
    }))
}

fn unavailable(body: Value) -> Response {
    let mut resp = (StatusCode::SERVICE_UNAVAILABLE, Json(body)).into_response();
    resp.headers_mut().insert(header::RETRY_AFTER, header::HeaderValue::from_static("5"));
    resp
}

// --- R41.7 (5): free-disk precheck ------------------------------------------

/// Bytes free for this user on the volume holding `dir` (`None` = unknown).
pub fn free_bytes(dir: &std::path::Path) -> Option<u64> {
    if let Some(v) = test_free_bytes_override() {
        return Some(v);
    }
    imp_free_bytes(dir)
}

/// Debug builds only: `ANCHOR_TEST_FREE_DISK_BYTES` fakes the free space so
/// the black-box suite can exercise the refusal.
fn test_free_bytes_override() -> Option<u64> {
    if !cfg!(debug_assertions) {
        return None;
    }
    std::env::var("ANCHOR_TEST_FREE_DISK_BYTES").ok()?.parse().ok()
}

#[cfg(unix)]
fn imp_free_bytes(dir: &std::path::Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(dir.as_os_str().as_bytes()).ok()?;
    // SAFETY: `c` is a valid NUL-terminated path; `st` is plain data the
    // call fills in, read only on success.
    unsafe {
        let mut st: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(c.as_ptr(), &mut st) != 0 {
            return None;
        }
        #[allow(clippy::unnecessary_cast)]
        Some(st.f_bavail as u64 * st.f_frsize as u64)
    }
}

#[cfg(windows)]
fn imp_free_bytes(dir: &std::path::Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    let wide: Vec<u16> = dir.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    let mut free: u64 = 0;
    // SAFETY: `wide` is NUL-terminated; the out-pointers are our locals or null.
    let ok = unsafe {
        windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut free,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    (ok != 0).then_some(free)
}

#[cfg(not(any(unix, windows)))]
fn imp_free_bytes(_dir: &std::path::Path) -> Option<u64> {
    None
}

/// The refusal text when `free` is below `need` (`None` = enough / unknown).
pub fn disk_shortfall(db_bytes: u64, free: Option<u64>) -> Option<String> {
    let need = db_bytes.saturating_mul(2);
    let free = free?;
    if free >= need {
        return None;
    }
    Some(format!(
        "Not enough free disk space to upgrade the database safely: the upgrade needs about {} free (twice the {} database, for the safety backup and table rewrites), and only {} is free. Free up space on that disk, then restart AnchorCore.",
        human_bytes(need),
        human_bytes(db_bytes),
        human_bytes(free)
    ))
}

pub fn human_bytes(n: u64) -> String {
    const GB: f64 = 1_000_000_000.0;
    const MB: f64 = 1_000_000.0;
    let f = n as f64;
    if f >= GB {
        format!("{:.1} GB", f / GB)
    } else {
        format!("{:.0} MB", (f / MB).max(1.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortfall_needs_twice_the_db() {
        assert!(disk_shortfall(1_000_000_000, Some(2_000_000_000)).is_none());
        assert!(disk_shortfall(1_000_000_000, None).is_none(), "unknown free space never refuses");
        let msg = disk_shortfall(5_500_000_000, Some(3_000_000_000)).unwrap();
        assert!(msg.contains("11.0 GB") && msg.contains("5.5 GB") && msg.contains("3.0 GB"), "{msg}");
    }

    #[test]
    fn agent_message_names_the_step_and_says_retry() {
        let msg = agent_message(&json!({"status":"starting","step":2,"of":4,"detail":"Migration 29: rewriting the search index"}));
        assert!(msg.contains("upgrading its database (step 2 of 4"), "{msg}");
        assert!(msg.contains("try again shortly"), "{msg}");
        let failed = agent_message(&json!({"status":"failed","error":"disk I/O error"}));
        assert!(failed.contains("could not upgrade") && failed.contains("disk I/O error"), "{failed}");
    }

    #[test]
    fn shell_paths_pass_only_for_reads() {
        assert!(is_shell_path("/", &Method::GET));
        assert!(is_shell_path("/assets/index-abc.js", &Method::GET));
        assert!(!is_shell_path("/sources", &Method::GET));
        assert!(!is_shell_path("/", &Method::POST));
    }

    #[test]
    fn free_bytes_reads_the_volume() {
        let dir = tempfile::tempdir().unwrap();
        if cfg!(any(unix, windows)) && std::env::var("ANCHOR_TEST_FREE_DISK_BYTES").is_err() {
            assert!(free_bytes(dir.path()).is_some_and(|n| n > 0));
        }
    }
}
