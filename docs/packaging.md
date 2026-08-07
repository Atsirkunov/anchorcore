# AnchorCore — Packaging Plan

> Goal: ship AnchorCore as a runnable app a non-developer can launch.
> Status: **Windows + macOS implemented (B20/B24)** — `dist/AnchorCore.exe` (Win,
> windowed — no console window) and `dist/AnchorCore.app` (macOS, ad-hoc signed,
> windowed — no Terminal window).

---

## 1. What We're Shipping (current)

| Piece | Form | Notes |
|---|---|---|
| API backend | Python + FastAPI (uvicorn in-process) | Single-file exe via PyInstaller |
| UI | Vite static build (`frontend/dist`) | Bundled into the exe; served at `127.0.0.1:8000` |
| Classifier/embed model | External Ollama | Auto-started by the exe; models pulled via Settings/sync |
| Data | SQLite + sqlite-vec | Per-user: `~/.anchorcore` (frozen mode) |

## 2. Building (Windows)

One command from the repo root:

```powershell
.\build.ps1
```

Steps it runs: `npm run build` → ensure venv + pyinstaller → `pyinstaller packaging.spec`
→ `dist/AnchorCore/` (onedir folder) → zip → `dist/AnchorCore-windows.zip`.

Spec details (`packaging.spec`):
- Bundles `frontend/dist` → `_MEIPASS/frontend_dist` (mounted by `main.py` when frozen)
- Bundles `backend/alembic/` + `alembic.ini` → startup migrations work
- Collects the `sqlite_vec` native DLL (PyInstaller doesn't auto-find it)
- **`console=False`** — the exe is a GUI-subsystem app, so no cmd window opens
- Entry: `backend/run_app.py` — sets `ANCHOR_DATA_DIR=~/.anchorcore` when frozen,
  auto-starts the local Ollama server, opens the browser, boots uvicorn

## 3. Building (macOS)

One command from the repo root:

```bash
./build.sh
```

Steps it runs: `npm run build` → ensure venv + pyinstaller → `pyinstaller packaging.spec`
→ `dist/AnchorCore.app` (windowed onedir macOS bundle) → `codesign --force --deep --sign -`
(ad-hoc, free — no Apple account needed) → `zip` → `dist/AnchorCore-macos.zip`.

The same `packaging.spec` works on both platforms: the sqlite_vec glob collects
the native lib (`*.dll` on Windows, `*.dylib` on macOS). On macOS the spec wraps
the executable in a `BUNDLE` (`AnchorCore.app`) so Finder launches it **without
opening Terminal** — double-clicking a bare binary would open one. Recipients
unzip once and drag `AnchorCore.app` to Applications. On first launch, Gatekeeper
shows "cannot be verified" — right-click → Open (or System Settings → Privacy &
Security → Open Anyway) approves it once.

## 4. Tester experience (first run)

1. Unzip → double-click `AnchorCore.exe` / `AnchorCore.app` (no terminal window)
2. Ollama starts (if installed), browser opens `http://127.0.0.1:8000`
3. Sources tab → connect a folder (e.g. the bundled `sample/`)
4. Sync; ask questions with citations

`~/.anchorcore/` holds the DB, logs, secrets. Deleting it = fresh start.
(Windowed builds have no stdout/stderr; `run_app.py` redirects them to devnull,
and logs land in `~/.anchorcore/anchorcore.log`.)

## 5. Open work (later)

| Item | Why | Effort |
|---|---|---|
| Ollama installer check | exe assumes Ollama present; add guided install/first-run (B8 overlap) | 0.5 day |
| macOS notarization | silent Gatekeeper approval; needs Apple Developer account ($99/yr) | 1–2 days |
| Windows installer (Inno Setup/NSIS) | nicer than a raw exe | 0.5 day |
| App icon | currently ships the generic PyInstaller icon | 0.5 day |
| Auto-update | Sparkle/tauri-updater | later |
| Bundled inference (llama.cpp) | fully offline, no Ollama prerequisite | +2–3 days |

## 6. Platform strategy

- Windows first (implemented). macOS next (implemented — windowed `.app` bundle,
  ad-hoc signed; notarization deferred). Linux after.
- One artifact per platform; data always per-user outside the binary.

---

*Companion docs: [product-plan.md](./product-plan.md), [architecture.md](./architecture.md)*
