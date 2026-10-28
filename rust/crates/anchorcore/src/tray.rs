//! R34.1: system tray (Windows + macOS) — status line (R41.7) + `Open AnchorCore` + `Quit AnchorCore`.
//!
//! Release builds are windowed (no console), so without a tray the app can only
//! be stopped from the Task Manager / Activity Monitor. The tray owns no state:
//! Open launches the browser at the running server, Quit triggers the graceful
//! shutdown in `main.rs` via the shared `Notify`. Closing the browser never
//! stops the server — tray Quit (or Ctrl+C / SIGTERM) is the only exit.
//!
//! Platform notes:
//! - Windows/macOS only: the `tray-icon` dependency is target-gated in
//!   `Cargo.toml`, so Linux/CI builds stay system-lib-free; there this module
//!   is a stub that logs once and returns `None`.
//! - The icon is created on the main thread (macOS AppKit): `main` binds via
//!   `std::net::TcpListener` (no await) before calling here. A second instance
//!   fails the bind before it can plant a zombie icon. `spawn` also initializes
//!   `NSApplication` first — without it `NSStatusBar` aborts (CGSConnection
//!   assertion, uncatchable; tray-icon never inits it). See `docs/tray.md`.
//! - Opt-out: `--no-tray` or `ANCHOR_TRAY=0` (headless/Team/CI).

use std::sync::Arc;

const ICON_W: u32 = 32;
const ICON_H: u32 = 32;

// favicon.svg (Stone & Sage): paper bg, taupe border + bars, pine spine.
const PAPER: [u8; 4] = [0xF2, 0xF0, 0xEB, 0xFF];
const TAUPE: [u8; 4] = [0x9A, 0x8B, 0x7A, 0xFF];
const PINE: [u8; 4] = [0x4A, 0x5A, 0x52, 0xFF];
// R41.6 macOS template image: AppKit draws only the alpha channel.
const INK: [u8; 4] = [0x00, 0x00, 0x00, 0xFF];
const CLEAR: [u8; 4] = [0x00, 0x00, 0x00, 0x00];

/// Shared with `maybe_open_browser`: absolute `/usr/bin/open` on macOS because
/// a Finder launch has a minimal PATH; the `open` crate everywhere else.
pub fn open_url(url: &str) -> Result<(), String> {
    if cfg!(target_os = "macos") {
        std::process::Command::new("/usr/bin/open")
            .arg(url)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string())
    } else {
        open::that(url).map_err(|e| e.to_string())
    }
}

/// Why the tray stays off (`None` = enabled). Env is injected so tests stay
/// hermetic — process env is global and cargo runs tests in threads.
fn disabled_reason(no_tray_flag: bool, anchor_tray_env: Option<&str>) -> Option<&'static str> {
    if no_tray_flag {
        return Some("--no-tray");
    }
    if anchor_tray_env == Some("0") {
        return Some("ANCHOR_TRAY=0");
    }
    if cfg!(any(target_os = "windows", target_os = "macos")) {
        None
    } else {
        Some("Windows/macOS only")
    }
}

/// R41.5: whether `main` should hand the main thread to the tray event loop
/// (and run the server on a background thread).
pub fn wanted(no_tray_flag: bool) -> bool {
    let env = std::env::var("ANCHOR_TRAY").ok();
    disabled_reason(no_tray_flag, env.as_deref()).is_none()
}

/// R41.5: pump the platform event loop on the main thread — without it the
/// icon never receives clicks (and macOS flags the app "Not Responding").
/// Never returns; the server thread ends the process after its drain.
pub fn run_event_loop(guard: TrayGuard) -> ! {
    imp::run_event_loop(&guard.0);
    loop {
        std::thread::park();
    }
}

/// Create the tray icon unless disabled (flag/env/platform). Failures log and
/// fall back to tray-less — the server must run even without a tray.
pub fn spawn_if_enabled(
    addr: &str,
    no_tray_flag: bool,
    shutdown: Arc<tokio::sync::Notify>,
) -> Option<TrayGuard> {
    let env = std::env::var("ANCHOR_TRAY").ok();
    if let Some(reason) = disabled_reason(no_tray_flag, env.as_deref()) {
        tracing::info!("tray disabled ({})", reason);
        return None;
    }
    imp::spawn(addr, shutdown)
}

pub struct TrayGuard(imp::Guard);

fn put(buf: &mut [u8], x: u32, y: u32, c: [u8; 4]) {
    let i = ((y * ICON_W + x) * 4) as usize;
    buf[i..i + 4].copy_from_slice(&c);
}

fn fill_rect(buf: &mut [u8], x0: u32, y0: u32, x1: u32, y1: u32, c: [u8; 4]) {
    for y in y0..y1 {
        for x in x0..x1 {
            put(buf, x, y, c);
        }
    }
}

fn in_rounded(x: i32, y: i32, x0: i32, y0: i32, x1: i32, y1: i32, r: i32) -> bool {
    if x < x0 || y < y0 || x >= x1 || y >= y1 {
        return false;
    }
    let cx = x.clamp(x0 + r, x1 - r - 1);
    let cy = y.clamp(y0 + r, y1 - r - 1);
    let dx = x - cx;
    let dy = y - cy;
    dx * dx + dy * dy <= r * r
}

fn fill_rounded(buf: &mut [u8], x0: u32, y0: u32, x1: u32, y1: u32, r: u32, c: [u8; 4]) {
    for y in y0..y1 {
        for x in x0..x1 {
            if in_rounded(
                x as i32, y as i32, x0 as i32, y0 as i32, x1 as i32, y1 as i32, r as i32,
            ) {
                put(buf, x, y, c);
            }
        }
    }
}

/// 32x32 RGBA raster of `frontend/public/favicon.svg` (halved coordinates):
/// taupe rounded border, paper inset, pine spine, three taupe bars.
pub fn build_icon_rgba() -> (Vec<u8>, u32, u32) {
    let mut buf = vec![0u8; (ICON_W * ICON_H * 4) as usize];
    fill_rounded(&mut buf, 1, 1, 31, 31, 7, TAUPE);
    fill_rounded(&mut buf, 3, 3, 29, 29, 5, PAPER);
    fill_rect(&mut buf, 8, 6, 13, 26, PINE);
    fill_rect(&mut buf, 16, 8, 24, 11, TAUPE);
    fill_rect(&mut buf, 16, 14, 22, 17, TAUPE);
    fill_rect(&mut buf, 16, 20, 24, 23, TAUPE);
    (buf, ICON_W, ICON_H)
}

/// R41.6: the same glyph as a monochrome template for the macOS menu bar —
/// a 2 px rounded outline, the spine and three bars in ink, the rest clear,
/// so AppKit tints it like native items in light and dark bars.
pub fn build_template_rgba() -> (Vec<u8>, u32, u32) {
    let mut buf = vec![0u8; (ICON_W * ICON_H * 4) as usize];
    fill_rounded(&mut buf, 1, 1, 31, 31, 7, INK);
    fill_rounded(&mut buf, 3, 3, 29, 29, 5, CLEAR);
    fill_rect(&mut buf, 8, 6, 13, 26, INK);
    fill_rect(&mut buf, 16, 8, 24, 11, INK);
    fill_rect(&mut buf, 16, 14, 22, 17, INK);
    fill_rect(&mut buf, 16, 20, 24, 23, INK);
    (buf, ICON_W, ICON_H)
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
mod imp {
    use super::Arc;
    use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem};
    use tray_icon::{Icon, TrayIconBuilder};

    pub(super) struct Guard {
        _icon: tray_icon::TrayIcon,
        _items: Vec<MenuItem>,
        #[cfg(target_os = "macos")]
        _app: objc2::rc::Retained<objc2_app_kit::NSApplication>,
    }

    /// macOS: `NSStatusBar.statusItemWithLength` aborts (CGSConnection assertion,
    /// uncatchable) unless NSApplication exists — tray-icon never creates it, so
    /// we do. Must run on the main thread (see `main`: pre-await, std bind).
    #[cfg(target_os = "macos")]
    fn init_appkit() -> Option<objc2::rc::Retained<objc2_app_kit::NSApplication>> {
        use objc2::MainThreadMarker;
        use objc2_app_kit::NSApplication;
        let Some(mtm) = MainThreadMarker::new() else {
            tracing::warn!("tray: not on the main thread, running without a tray icon");
            return None;
        };
        Some(NSApplication::sharedApplication(mtm))
    }

    /// macOS: template image (tinted like native items) and a saved position,
    /// so an icon ⌘-dragged out from under long menus stays put (R41.6).
    #[cfg(target_os = "macos")]
    fn with_platform_icon(b: TrayIconBuilder, icon: Icon) -> TrayIconBuilder {
        b.with_icon_templated(icon).with_autosave_name("AnchorCore")
    }

    #[cfg(not(target_os = "macos"))]
    fn with_platform_icon(b: TrayIconBuilder, icon: Icon) -> TrayIconBuilder {
        b.with_icon(icon)
    }

    pub(super) fn spawn(
        addr: &str,
        shutdown: Arc<tokio::sync::Notify>,
    ) -> Option<super::TrayGuard> {
        #[cfg(target_os = "macos")]
        let app = init_appkit()?;
        let url = format!("http://{}", addr);
        // R41.7: a disabled first line follows the startup status.
        let status_item = MenuItem::new(status_text(), false, None);
        let open_item = MenuItem::new("Open AnchorCore", true, None);
        let quit_item = MenuItem::new("Quit AnchorCore", true, None);
        let menu = Menu::new();
        if menu.append(&status_item).is_err() || menu.append(&open_item).is_err() || menu.append(&quit_item).is_err() {
            tracing::warn!("tray: menu build failed, running without a tray icon");
            return None;
        }
        let (rgba, w, h) =
            if cfg!(target_os = "macos") { super::build_template_rgba() } else { super::build_icon_rgba() };
        let icon = match Icon::from_rgba(rgba, w, h) {
            Ok(icon) => icon,
            Err(e) => {
                tracing::warn!("tray: icon build failed ({}), running without one", e);
                return None;
            }
        };
        tracing::info!("creating system tray icon");
        let tray = match with_platform_icon(TrayIconBuilder::new(), icon)
            .with_menu(Box::new(menu))
            .with_tooltip(crate::startup::tray_label())
            .with_menu_on_left_click(true)
            .build()
        {
            Ok(tray) => tray,
            Err(e) => {
                tracing::warn!("tray: failed to create icon ({}), running without one", e);
                return None;
            }
        };
        let open_id = open_item.id().clone();
        let quit_id = quit_item.id().clone();
        if std::thread::Builder::new()
            .name("tray-menu".to_string())
            .spawn(move || event_loop(&url, &open_id, &quit_id, &shutdown))
            .is_err()
        {
            tracing::warn!("tray: menu thread failed to start, running without a tray icon");
            return None;
        }
        tracing::info!("tray ready at {} (Open + Quit)", format!("http://{}", addr));
        STATUS_UI.with(|ui| *ui.borrow_mut() = Some((tray.clone(), status_item.clone())));
        crate::startup::on_change(Box::new(schedule_refresh));
        Some(super::TrayGuard(Guard {
            _icon: tray,
            _items: vec![status_item, open_item, quit_item],
            #[cfg(target_os = "macos")]
            _app: app,
        }))
    }

    thread_local! {
        /// R41.7: the icon + status line, owned by the main thread (AppKit /
        /// Win32 objects are not `Send`); refreshed only there.
        static STATUS_UI: std::cell::RefCell<Option<(tray_icon::TrayIcon, MenuItem)>> =
            const { std::cell::RefCell::new(None) };
    }

    fn status_text() -> String {
        crate::startup::tray_label().trim_start_matches("AnchorCore — ").replace("AnchorCore", "Running")
    }

    /// Main thread only: copy the startup status into tooltip + status line.
    fn refresh_status() {
        STATUS_UI.with(|ui| {
            if let Some((tray, item)) = ui.borrow().as_ref() {
                let _ = tray.set_tooltip(Some(crate::startup::tray_label()));
                item.set_text(status_text());
            }
        });
    }

    /// macOS: hop onto the main queue (drained by `NSApplication.run`).
    #[cfg(target_os = "macos")]
    fn schedule_refresh() {
        dispatch2::DispatchQueue::main().exec_async(refresh_status);
    }

    /// Windows: the pump below refreshes on its 1 s timer.
    #[cfg(target_os = "windows")]
    fn schedule_refresh() {}

    /// macOS: `NSApplication.run` delivers status-item clicks to muda, which
    /// forwards them to the `tray-menu` thread below.
    #[cfg(target_os = "macos")]
    pub(super) fn run_event_loop(guard: &Guard) {
        guard._app.run();
    }

    /// Windows: tray-icon's hidden window lives on this thread; a plain
    /// message pump dispatches its clicks. Returns on `WM_QUIT` or error.
    #[cfg(target_os = "windows")]
    pub(super) fn run_event_loop(_guard: &Guard) {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            DispatchMessageW, GetMessageW, SetTimer, TranslateMessage, MSG, WM_TIMER,
        };
        // SAFETY: MSG is plain data; the pump only reads/writes our local.
        // The thread timer (no window, no callback) only posts WM_TIMER here.
        unsafe {
            // R41.7: refresh the startup status once a second.
            SetTimer(std::ptr::null_mut(), 0, 1000, None);
            let mut msg: MSG = std::mem::zeroed();
            while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                if msg.message == WM_TIMER && msg.hwnd.is_null() {
                    refresh_status();
                    continue;
                }
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }

    fn event_loop(
        url: &str,
        open_id: &MenuId,
        quit_id: &MenuId,
        shutdown: &Arc<tokio::sync::Notify>,
    ) {
        while let Ok(event) = MenuEvent::receiver().recv() {
            if event.id == *open_id {
                if let Err(e) = super::open_url(url) {
                    tracing::warn!("tray: failed to open browser at {}: {}", url, e);
                }
            } else if event.id == *quit_id {
                tracing::info!("tray: Quit requested");
                shutdown.notify_one();
                break;
            }
        }
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
mod imp {
    use super::Arc;

    pub(super) struct Guard;

    pub(super) fn spawn(
        _addr: &str,
        _shutdown: Arc<tokio::sync::Notify>,
    ) -> Option<super::TrayGuard> {
        // Unreachable: disabled_reason() already filtered this platform out.
        // Kept as a no-op (not unreachable!()) so a future Linux tray lands here.
        None
    }

    pub(super) fn run_event_loop(_guard: &Guard) {}
}

#[cfg(test)]
mod tray_tests {
    use super::*;

    fn px(buf: &[u8], x: u32, y: u32) -> [u8; 4] {
        let i = ((y * ICON_W + x) * 4) as usize;
        [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]
    }

    #[test]
    fn gating_flag_and_env_win_on_every_platform() {
        assert_eq!(disabled_reason(true, None), Some("--no-tray"));
        assert_eq!(disabled_reason(true, Some("0")), Some("--no-tray"));
        assert_eq!(disabled_reason(false, Some("0")), Some("ANCHOR_TRAY=0"));
        // Any other value (including unset and garbage) leaves the env gate open.
        assert_ne!(disabled_reason(false, Some("1")), Some("ANCHOR_TRAY=0"));
        assert_ne!(disabled_reason(false, Some("off")), Some("ANCHOR_TRAY=0"));
    }

    #[test]
    fn gating_defaults_to_platform() {
        if cfg!(any(target_os = "windows", target_os = "macos")) {
            assert_eq!(disabled_reason(false, None), None);
        } else {
            assert_eq!(disabled_reason(false, None), Some("Windows/macOS only"));
        }
    }

    #[test]
    fn icon_is_32x32_rgba_matching_the_favicon() {
        let (buf, w, h) = build_icon_rgba();
        assert_eq!((w, h), (32, 32));
        assert_eq!(buf.len(), 32 * 32 * 4);
        assert_eq!(px(&buf, 0, 0)[3], 0, "outer corner transparent");
        assert_eq!(px(&buf, 1, 1)[3], 0, "rounded corner transparent");
        assert_eq!(px(&buf, 2, 15), TAUPE, "left border");
        assert_eq!(px(&buf, 25, 25), PAPER, "paper inset");
        assert_eq!(px(&buf, 10, 10), PINE, "spine");
        assert_eq!(px(&buf, 20, 10), TAUPE, "top bar");
        assert_eq!(px(&buf, 18, 15), TAUPE, "middle bar");
        assert_eq!(px(&buf, 20, 21), TAUPE, "bottom bar");
    }

    #[test]
    fn template_icon_is_alpha_only_glyph() {
        let (buf, w, h) = build_template_rgba();
        assert_eq!((w, h, buf.len()), (32, 32, 32 * 32 * 4));
        assert!(buf.chunks(4).all(|p| p == INK || p == CLEAR), "only ink or clear pixels");
        assert_eq!(px(&buf, 0, 0), CLEAR, "outer corner");
        assert_eq!(px(&buf, 2, 15), INK, "outline");
        assert_eq!(px(&buf, 25, 25), CLEAR, "inside is see-through");
        assert_eq!(px(&buf, 10, 10), INK, "spine");
        assert_eq!(px(&buf, 20, 10), INK, "top bar");
        assert_eq!(px(&buf, 14, 10), CLEAR, "gap between spine and bars");
    }
}
