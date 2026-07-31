# AnchorCore — macOS Packaging Plan (draft)

> Goal: ship AnchorCore as a distributable macOS desktop app.
> Status: plan only — no implementation yet.

---

## 1. What We're Shipping

Three runtime pieces today:

| Piece | Current form | Ship form |
|---|---|---|
| API backend | Python + FastAPI (uvicorn) | Native binary (`PyInstaller`) |
| UI | Next.js dev server | Static build served by backend |
| Classifier model | External Ollama | Bundled inference **or** Ollama prerequisite |

SQLite is already embedded — no external database runtime to ship.

## 2. Recommended Approach (Option B — pragmatic, offline-capable path first)

**Backend → single binary**
- `PyInstaller` bundles Python, FastAPI, SQLAlchemy, pypdf, httpx, and app code into one `.app`.
- uvicorn runs in-process; the app serves both the API and the static UI on `127.0.0.1`.

**Frontend → static export**
- `next build` with `output: "export"` produces plain HTML/JS/CSS.
- FastAPI serves it as static files. No Node runtime in the shipped product.

**Model → first launch checks for Ollama**
- App detects Ollama at startup; if missing, offers guided install (`ollama pull llama3.2`).
- No inference code changes — the OpenAI-compatible endpoint already works.

**Data directory**
- SQLite DB + uploads under `~/Library/Application Support/AnchorCore/`.

## 3. Model Strategy Decision (pick later, affects scope)

| Option | Self-contained | Effort | Notes |
|---|---|---|---|
| **B. Ship with Ollama** | No (one prerequisite) | ~2–3 days total | Zero inference changes; recommended first |
| **A. Embedded llama.cpp** | Yes, fully offline | +2–3 days | Bundle `llama3.2:3b` GGUF (~2GB); replace HTTP classifier with local inference |
| **C. Cloud API only** | No (needs key) | Lowest | Contradicts offline/privacy positioning; not recommended |

Keep the `Classifier` abstraction so the inference backend can swap without touching the API surface.

## 4. macOS-Specific Work (1–3 days)

- **Code signing + notarization** — required for Gatekeeper to run the `.app` normally. Requires Apple Developer account ($99/yr).
- **First-launch flow** — model location check, data dir creation, port binding.
- **Menu bar / tray** — optional polish; app runs as localhost server with browser UI.
- **Auto-update** — defer; later via Sparkle or Tauri updater.

## 5. Platform Strategy

- `PyInstaller` builds are per-OS → separate artifacts for macOS, Windows, Linux.
- Mac = primary target (per this plan); Windows/Linux later.

## 6. Deliverables / Milestones

| Milestone | Output | Rough effort |
|---|---|---|
| 1. Static UI + served by FastAPI | Local dev runs UI from backend | 0.5 day |
| 2. PyInstaller `.app` | Launchable app bundle | 1 day |
| 3. First-launch + Ollama check | Guided setup flow | 1 day |
| 4. Sign + notarize + `.dmg` | Distributable | 1–2 days |
| 5. (Later) Embedded inference | Fully offline app | +2–3 days |

## 7. Open Questions

- [ ] Target macOS version floor (10.15 Catalina+ vs Sonoma+)?
- [ ] Ship as `.app` zip or `.dmg`?
- [ ] Apple Developer account in place?
- [ ] Do we bundle Ollama at all, or link to installer?
- [ ] Windows/Linux timing?

---

*Companion docs: [product-plan.md](./product-plan.md), [architecture.md](./architecture.md)*
