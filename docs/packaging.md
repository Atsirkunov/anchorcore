# AnchorCore — Packaging Plan

> Goal: ship AnchorCore as a runnable app a non-developer can launch.
> Status: **Rust shipped** (current version: see `docs/release-notes/`) — `rust/target/release/anchorcore` release binary (`frontend/dist` embedded via `include_dir!`, ad-hoc `codesign`) is the local artifact. **Python `backend/` PyInstaller (`packaging.spec`) is archived on tag `archive/python-final`** (conformance suite fetches it frozen). Legacy `dist/AnchorCore.exe/.app` (B20/B24) remain documented for history.

---

## 1. What We're Shipping (current — Rust)

| Piece | Form | Notes |
|---|---|---|
| API backend | **Rust Axum** — app + `anchorcore mcp` mode in one binary (R20.8) | `cargo build --release` → `rust/target/release/anchorcore` (`frontend/dist` embedded; one file per OS zip) |
| UI | Vite static build (`frontend/dist`) | Embedded via `rust/crates/anchorcore/src/frontend.rs:1` `include_dir!`; served at `127.0.0.1:8000` |
| Classifier/embed model | External Ollama | Auto-started; models pulled via Settings/sync |
| Data | SQLite (FTS5 + embedding scan) | Per-user: `~/.anchorcore` |
| Legacy Python | FastAPI PyInstaller (on the archive tag) | **Archived** — hosting image fetches it frozen |

## 2. Building (Rust — shipped)

From the repo root:

```bash
cd frontend && npm run build && cd ..
cargo build --release
# artifact: rust/target/release/anchorcore (frontend/dist embedded; `anchorcore mcp` is a mode)
# release zips: dist/AnchorCore-{windows,macos,linux}.zip via release.yml
```

`release.yml` builds `AnchorCore-{windows,macos,linux}.zip` on every `v*` tag (`frontend` → `cargo build --release` → `codesign`).

## 3. Historical: Python builds (archived on tag `archive/python-final`)

Archived — the spec, `backend/`, and start scripts are frozen on the tag; the hosting image fetches that snapshot. Do not use for local dev:

```powershell
.\build.ps1   # Windows legacy
./build.sh    # macOS legacy
```

Steps (legacy `packaging.spec`): `npm run build` → venv + pyinstaller → `pyinstaller packaging.spec` → `dist/AnchorCore/` / `.app` → `codesign --force --deep --sign -` (ad-hoc). Bundles `frontend/dist` → `_MEIPASS/frontend_dist`, `backend/alembic/`, `sqlite_vec` DLL/dylib, `console=False`, entry `backend/run_app.py`.

The same `packaging.spec` works on both platforms: the sqlite_vec glob collects
the native lib (`*.dll` on Windows, `*.dylib` on macOS). On macOS the spec wraps
the executable in a `BUNDLE` (`AnchorCore.app`) so Finder launches it **without
opening Terminal** — double-clicking a bare binary would open one. Recipients
unzip once and drag `AnchorCore.app` to Applications. On first launch, Gatekeeper
shows "cannot be verified" — System Settings → Privacy & Security → Open Anyway approves it once
(right-click → Open no longer bypasses Gatekeeper on recent macOS).

## 4. Tester experience (first run)

1. Unzip → double-click `anchorcore.exe` / `AnchorCore.app` (no Terminal window on either; panics append to `anchorcore.log`)
2. Ollama starts (if installed), browser opens `http://127.0.0.1:8000`
3. Sources tab → connect a folder (e.g. `sample/` from the repo)
4. Sync; ask questions with citations

`~/.anchorcore/` holds the DB, logs, secrets. Deleting it = fresh start.
(Windowed builds have no stdout/stderr — panics append to the log file instead,
and logs land in `~/.anchorcore/anchorcore.log`.)

## 5. Open work (later)

| Item | Why | Effort |
|---|---|---|
| macOS notarization | silent Gatekeeper approval; needs Apple Developer account ($99/yr) | 1–2 days |
| Windows installer (Inno Setup/NSIS) | nicer than a raw exe | 0.5 day |
| App icon | neither `.app` nor exe bundles a custom icon yet | needs a designer (or reuse `frontend/public/vite.svg` as placeholder) |
| Auto-update | Sparkle/tauri-updater | later |
| Bundled inference (llama.cpp) | fully offline, no Ollama prerequisite | +2–3 days |

## 6. Platform strategy

- Windows first (implemented). macOS next (implemented — windowed `.app` bundle,
  ad-hoc signed; notarization deferred). Linux zips ship in CI and are offered on the site (OS-aware download, 2026-10-21); Windows and Linux builds are not code-signed.
- One artifact per platform; data always per-user outside the binary.

---

*Companion docs: [product-plan.md](./product-plan.md), [architecture.md](./architecture.md)*
