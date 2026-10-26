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
  click path is exercised by hand on release builds (macOS confirmed
  2026-10-26 after R41.5; check the main thread with `sample <pid> 1` — it
  must sit in `-[NSApplication run]`).
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
- The main thread belongs to the tray (R41.5). With a tray enabled, `main`
  (sync, no `#[tokio::main]`) runs the server on a `server` thread
  (`run_with_tray`); after the bind succeeds the server sends `(addr,
  shutdown)` back over a channel, and the main thread creates the icon and
  pumps the platform event loop — `NSApplication.run` on macOS, a
  `GetMessageW` loop on Windows (`tray::run_event_loop`, never returns).
  AppKit/Win32 only draw the status item and deliver its clicks on a pumped
  main/creating thread: before R41.5 the main thread sat in `axum::serve`, so
  the macOS icon never appeared or took clicks and the app showed
  "(Not Responding)". Quit → `Notify` → graceful drain → the server thread
  calls `process::exit`. A second instance still fails the bind before any
  icon exists. Tray-less runs (`--no-tray`, `ANCHOR_TRAY=0`, Linux, CLI, MCP)
  keep `block_on(serve)` on the main thread.
- Finding the icon: macOS adds new status items on the far left; on a notched
  Mac with many items, long app menus can cover it. Look left of the existing
  icons (with Finder frontmost) and ⌘-drag it right. R41.6 tracks a
  template-image icon.
- Failures (not on main thread, no display, menu thread) log and fall back to
  tray-less; the server always runs.
