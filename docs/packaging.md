# AnchorCore — Packaging Plan

> Goal: ship AnchorCore as a runnable app a non-developer can launch.
> Status: **Windows + macOS implemented (B20/B24)** — `dist/AnchorCore.exe` (Win), `dist/AnchorCore` (macOS, ad-hoc signed).

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
→ `dist/AnchorCore.exe` (~24 MB, single file).

Spec details (`packaging.spec`):
- Bundles `frontend/dist` → `_MEIPASS/frontend_dist` (mounted by `main.py` when frozen)
- Bundles `backend/alembic/` + `alembic.ini` → startup migrations work
- Collects the `sqlite_vec` native DLL (PyInstaller doesn't auto-find it)
- Entry: `backend/run_app.py` — sets `ANCHOR_DATA_DIR=~/.anchorcore` when frozen,
  auto-starts the local Ollama server, opens the browser, boots uvicorn

## 3. Building (macOS)

One command from the repo root:

```bash
./build.sh
```

Steps it runs: `npm run build` → ensure venv + pyinstaller → `pyinstaller packaging.spec`
→ `dist/AnchorCore` (single-file, ~24 MB) → `codesign --force --deep --sign -`
(ad-hoc, free — no Apple account needed).

The same `packaging.spec` works on both platforms: the sqlite_vec glob collects
the native lib (`*.dll` on Windows, `*.dylib` on macOS). On first launch,
Gatekeeper shows "cannot be verified" — right-click → Open (or System Settings →
Privacy & Security → Open Anyway) approves it once.

## 4. Tester experience (first run)

1. Double-click `AnchorCore.exe`
2. Ollama starts (if installed), browser opens `http://127.0.0.1:8000`
3. Sources tab → connect a folder (e.g. the bundled `sample/`)
4. Sync; ask questions with citations

`~/.anchorcore/` holds the DB, logs, secrets. Deleting it = fresh start.

## 5. Open work (later)

| Item | Why | Effort |
|---|---|---|
| `console=False` in the spec | hide the terminal window (cosmetic) | 10 min |
| Ollama installer check | exe assumes Ollama present; add guided install/first-run (B8 overlap) | 0.5 day |
| macOS `.app` bundle + notarization | Gatekeeper silent approval; needs Apple Developer account ($99/yr) | 1–2 days |
| Windows installer (Inno Setup/NSIS) | nicer than a raw exe | 0.5 day |
| Auto-update | Sparkle/tauri-updater | later |
| Bundled inference (llama.cpp) | fully offline, no Ollama prerequisite | +2–3 days |

## 6. Platform strategy

- Windows first (implemented). macOS next (implemented — same spec, ad-hoc
  signed; `.app` bundle + notarization deferred). Linux after.
- One exe per platform; data always per-user outside the binary.

---

*Companion docs: [product-plan.md](./product-plan.md), [architecture.md](./architecture.md)*
