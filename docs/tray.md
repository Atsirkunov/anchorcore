# System tray (Windows + macOS)

Release builds are windowed (no console/terminal), so without a tray the only
way to stop the app is the Task Manager / Activity Monitor / `kill`. The tray
icon fixes that: it lives in the Windows system tray / macOS menu bar with two
items:

- **Open AnchorCore** — opens the app in the browser (`http://127.0.0.1:<port>`).
- **Quit AnchorCore** — graceful shutdown (in-flight requests drain, then exit).

Closing the browser tab never stops the server — Quit (or Ctrl+C / SIGTERM) is
the only exit, same as before.

## Turning it off

- Flag: `anchorcore --no-tray`
- Env: `ANCHOR_TRAY=0`
- Linux builds have no tray (stub) — nothing to turn off. Same for
  `anchorcore mcp`, `anchorcore export|import`, and the Team container.

## Notes

- The icon matches `frontend/public/favicon.svg` (the original Stone & Sage tile — the favicon predates the Archive palette), rasterized
  in code (`tray.rs: build_icon_rgba`) so the binary needs no image asset.
- Menu-bar clicks can't be automated here: the Open/Quit plumbing is
  unit-tested at the seams (gating matrix, icon pixels, Notify→shutdown) and
  creation + serving + graceful shutdown are smoke-tested live; the physical
  click path is exercised by hand on release builds.
- Headless macOS (SSH with no GUI session at all) is untested — if the app
  aborts there, `--no-tray` is the escape hatch.

## Maintainer notes (why it looks like this)

- `tray-icon` (Tauri) + its `muda` menu, target-gated to Windows/macOS so Linux
  CI needs no system tray libraries (`tray.rs` is a stub elsewhere).
- macOS: `NSApplication` MUST be initialized before `NSStatusBar` builds the
  item — without it macOS 15 aborts in `CGSConnectionByID` (uncatchable
  assertion, exit 134; `tray-icon` never inits it). Proven 2026-10-12: a bare
  20-line `statusItemWithLength` probe aborted while the Swift
  `NSApplication.shared` + status-item probe on the same machine succeeded.
  `init_appkit` mirrors the Swift probe exactly (shared instance only, no
  activation-policy change).
- The icon is created on the main thread: `main` binds via
  `std::net::TcpListener` (no await) before `spawn_if_enabled`, so tray
  creation never leaves the main thread — and a second instance fails the bind
  before it can plant a zombie icon.
- Failures (not on main thread, no display, menu thread) log and fall back to
  tray-less; the server always runs.
