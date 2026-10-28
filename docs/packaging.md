# AnchorCore — Packaging

> One binary per OS: the Rust app with `frontend/dist` embedded (`include_dir!`, `frontend.rs`) and the
> MCP server as a mode of the same binary (`anchorcore mcp`). Release steps: [releasing.md](./releasing.md).

## 1. What ships

| Piece | Form | Notes |
|---|---|---|
| App | Rust Axum binary | `cargo build --release` → `rust/target/release/anchorcore`; `anchorcore mcp` is a mode, not a second file |
| UI | Vite build (`frontend/dist`) | embedded at build time — run `npm run build` first or the binary ships stale UI |
| Models | none bundled | optional Ollama (auto-started when installed) or an OpenAI-compatible endpoint, for embeddings and local-mode answers |
| Data | SQLite (FTS5 + embedding scan) | per user in `~/.anchorcore` (DB, logs, secrets fallback) |

## 2. Building

```bash
cd frontend && npm run build && cd ..
cargo build --release      # from rust/ (or ./build.sh / .\build.ps1 from the repo root)
```

`release.yml` builds `AnchorCore-{windows,macos,linux}.zip` + `AnchorCore-macos.dmg` on every `v*` tag
(frontend → release build → ad-hoc `codesign` on macOS → DMG). Windows builds use the MinGW (GNU) toolchain — see `AGENTS.md`.

macOS ships an `AnchorCore.app` bundle so Finder launches it without a Terminal window, packed as
`AnchorCore-macos.dmg` (the app + an `/Applications` symlink to drag onto; R41.2) and, for now, also as
`AnchorCore-macos.zip`. It is ad-hoc signed, not notarized: first launch needs System Settings → Privacy &
Security → Open Anyway. Windows and Linux builds are not code-signed.

## 3. First run (tester view)

1. macOS: open the DMG, drag `AnchorCore.app` onto Applications, open it from `/Applications`. Windows/Linux:
   unzip → double-click `anchorcore.exe` / run `anchorcore`.
2. Ollama starts if installed; the browser opens `http://127.0.0.1:8000`; the tray/menu-bar icon keeps the
   app running (`tray.md`).
3. The onboarding wizard (R45.6) asks who writes answers (agent or local model) and sets up search
   embeddings (the built-in model, one ~550 MB download, or Ollama's `nomic-embed-text` when it runs).
   Local mode then picks an Ollama answer model sized to the machine's RAM (`GET /system/onboarding` →
   `machine`); agent mode picks a channel — MCP Connect or the REST `ask-docs` skill. Then it
   connects a folder (or the bundled sample).
4. Sync, then ask — answers come back with citations.

Windowed builds have no console: startup errors and panics go to `~/.anchorcore/anchorcore.log`
(console output for terminal runs is B51). Deleting `~/.anchorcore` is a fresh start.

## 4. Open work

| Item | Tracked as |
|---|---|
| DMG with drag-to-Applications | R41.2 |
| Apple notarization (needs a paid developer account) | R41.4 (owner call) |
| Built-in embedder (no Ollama needed for hybrid search) | R45.3 → R45.4 |
| Sample corpus inside the release | B50 |
| Windows installer, custom app icon, auto-update | not scheduled |

---

*Companion docs: [releasing.md](./releasing.md), [tray.md](./tray.md), [updates.md](./updates.md), [architecture.md](./architecture.md)*
