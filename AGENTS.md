# AnchorCore — agent guide (living file)

Read this before doing anything. It is the distilled backend/architecture logic so
a fresh session doesn't have to re-explore the whole repo. Update it whenever you
learn something that would have helped at the start of a session.

## What this is

Local-first "company memory": connect sources (folder, Jira) → classify into
entities with provenance → hybrid RAG Q&A with citations. React SPA served by one
FastAPI process. Stack: Python 3.12 + FastAPI + SQLite (sqlite-vec + FTS5) ·
React/Vite · Ollama local + BYO cloud OpenAI-compatible. Packaged via PyInstaller.

Repo: `git@github.com:Atsirkunov/anchorcore.git`. Backend under `backend/`, UI under
`frontend/`. Release cadence: bump version → tag `vX.Y.Z` → push (CI builds artifacts).

## How to run / test / release

- Dev: `./start.sh` (venv, migrations, Ollama, backend :8000)
- Backend tests: `PYTHONPATH=backend backend/.venv/bin/python -m pytest backend/tests -q` (121 tests, ~25s; CI-safe, no LLM needed)
- Frontend: `cd frontend && npm run build` (tsc + vite; must pass before a UI change is done)
- Release: bump `__version__` in `backend/app/config.py` (single source of truth; main.py + system.py import it) → commit → tag → push
- **The packaged app embeds `frontend/dist` at build time** — after any UI change you must rebuild, or the exe ships stale UI.

## Backend architecture map

- `backend/app/main.py` — wires everything: `secrets`, `SettingsService`, `Classifier`, `Embedder`,
  `IngestionPipeline`, `AnswerEngine`, `Scheduler`, `JobManager`; runs Alembic migrations in lifespan;
  serves bundled UI last (must stay last).
- `backend/app/models.py` — SQLAlchemy models: `Source` (`label` B39), `IngestedItem`, `Entity` (`window_text`/`window_index` B26, `dispute_count` B3), `Relationship`, `Chunk` (`kind`: document|entity|distilled B18, `is_pii`/`pii_categories` B30), `Job`, `Project` + `project_sources`, `SystemEvent`, `AppSetting`, `MergeAction`, `User` (B40 hosted auth). `content_hash()` helper (re-exported via `hashing.py`).
- `backend/app/schemas.py` — Pydantic request/response models. `SourceOut` does NOT include `config`
  (config is fetched via `GET /sources/{id}/config` which masks secrets); `SourceCreate/Update` validate `label` (B39).
- `backend/app/answer_engine.py` — **the core retrieval pipeline** (see below). `vec_chunks` vec0 index (B33) with Python-scan fallback; gates `sensitive`/`pii` + `is_pii` chunks from cloud answer when `ANCHOR_CLOUD_TRUST` missing (B30).
- `backend/app/pipeline.py` — ingestion orchestration: classify (windowed, hash-skipped, concurrency-limited),
  distill (B18), embed (IDF-gated, `is_pii` chunks skip cloud embed B30, `_flag_pii`), PII gate (B39). Chunking split to `chunking.py`/`hashing.py`/`distill.py` (B35).
- `backend/app/chunking.py` / `hashing.py` / `distill.py` — extracted from `pipeline.py` (B35).
- `backend/app/pii.py` — PII knowledge base + `scan_text` (B30), categories + custom words, persisted in `app_settings`.
- `backend/app/auth.py` — hosted auth (B40): PBKDF2 + HS256 JWT, `get_current_user`/`require_auth` (off when `ANCHOR_AUTH_SECRET` empty).
- `backend/app/classifier.py` — per-doc-type extraction prompts, `detect_document_type`, `classify`,
  `distill`; rule-based fallbacks for all (tests must pass without Ollama). Takes `cloud_trusted` (B39).
- `backend/app/routers/` — `sources.py` (create/list/update/delete + config + jobs, label B39),
  `entities.py` (`GET /{id}`, `PATCH`, `GET /{id}/related`, `POST /{id}/dispute`, `GET /{id}/disputes`, **`GET /{id}/context` B27 neighbour expansion**; `review_router` for low-confidence/duplicates/merge), `qa.py` (`POST /qa` + `POST /qa/public` B30 share-safe), `projects.py`, `settings.py`, `system.py` (status/errors/logs/onboarding B8), `pii.py` (`/pii/config`, `/pii/review`, `/pii/review/{id}`, `/pii/scan/{source}`), `auth.py` (`/auth/status`, `/auth/signup`, `/auth/login`, `/auth/me` B40).
- `backend/app/secrets.py` — `SecretStore` (OS keychain, encrypted-file fallback when
  `ANCHOR_SECRETS_NO_KEYRING=1`); `SECRET_SOURCE_FIELDS` single source of truth (B34); `store_source_config`/`resolve_source_config` handle secret fields. Never ship secrets to the UI.
- `backend/app/app_settings.py` — `SettingsService`: env `.env` is default, DB `app_settings` overrides
  win, secrets resolve from SecretStore. Read effective values at call time (no restart). `SETTING_KEYS`/
  `SECRET_KEYS`/`ALL_KEYS` control what the Settings UI exposes.
- `backend/app/config.py` — single source `__version__` (B34) + pydantic `Settings` (env prefix `ANCHOR_`, `.env` file). Adds `auth_secret`/`auth_token_hours`/`auth_enabled` (B40), `cloud_trust` helpers (B30/B39), retrieval knobs, `embed_dim` (B33 vec0).
- `backend/app/db.py` — engine/session; **sets `PRAGMA foreign_keys=ON` (critical, deletions depend on it)**. `engine` is env-driven (`resolved_database_url` → SQLite local, `postgresql+psycopg` hosted via `hosting/`).
- `backend/alembic/versions/` — migrations; chain head is now `b40a0c1` (users B40). New migrations must set
  `down_revision` to the current head. Convention: one migration + matching model change per feature. SQLite-only migrations (`b12f7c0` FTS5, `b33a0c1` vec0) skip on Postgres.
- `backend/tests/` — helpers live in `tests/test_smoke.py`: `start_and_wait(client, source_id, kind="sync")`
  and `wait_job(...)`. Copy the `_mk_source(...)` pattern (create folder → sync → return source dict) from
  `test_projects.py`/`test_disputes.py` instead of hand-rolling sync flow in every test. Now 121 tests (see `tests/README.md`). Shared DB + optional `isolated_db` fixture (B38).

## AnswerEngine retrieval pipeline (the core logic)

`ask()` flow: resolve project source_ids (B15) → optionally rewrite follow-up via LLM → **planner**
`_plan_tools` (deterministic: `hybrid` always; `who_knows` added when query matches
who/whom/owns/owner/responsible/expert/knows) → **executor** `_execute_tools` (one shared embed call;
hybrid = vector + FTS5; who_knows) → `_fuse_evidence` (RRF fusion) → `_graph_expand` (B32, 1–2 hop
relationship walk, stale excluded) → generate with citations.

Key rules:
- RRF: `score = Σ weight/(60+rank)`. `_rrf_fuse` (2 lists) / `_rrf_fuse_multi` (N lists).
- Age decay `0.5^(age/halflife)`; per-item diversity cap (default 3) relaxed for single-file corpora;
  near-duplicate chunk dedupe by `_content_signature`.
- **Stale exclusion**: every retrieval path filters `entity.status == "stale"` via `AnswerEngine._status_ok`
  (vector, FTS, who_knows, fallback, graph). B3 added the same hook for `disputed` (excluded by default,
  toggle `ANCHOR_QA_EXCLUDE_DISPUTED`). If you add a new status-based exclusion, add it to `_status_ok`,
  not to individual tools.
- All retrieval methods take `source_ids: set[int] | None` (None = everything; B15 scoping).
- Evidence hit shape: `{"chunk", "entity", "item", "source_id", "score"}` (+ optional `"graph": True`,
  `"expanded"`).

## Packaging (PyInstaller)

- **onedir + `console=False`** in `packaging.spec` — windowed GUI app, **no terminal window**.
  macOS: EXE→`COLLECT`→`BUNDLE` yields `dist/AnchorCore.app`; Windows: `COLLECT` yields
  `dist/AnchorCore/` (zip the folder). Do NOT go back to onefile — PyInstaller rejects
  onefile+`.app` from v7.0.
- `dist/AnchorCore-macos.zip` (macOS) / `dist/AnchorCore-windows.zip` (Windows) are the ship artifacts;
  `build.sh`/`build.ps1` produce them; `release.yml` rebuilds on every `v*` tag.
- **`backend/run_app.py`** is the frozen entry: `_guard_windowed_stdio()` redirects stdout/stderr to
  devnull before importing `app.main` (windowed builds have no stdio — printing/logging would crash).
  Real logs go to `ANCHOR_DATA_DIR/anchorcore.log`.
- macOS CI must use **Homebrew Python** (loadable sqlite extensions) or sqlite_vec crashes at startup.
- Ad-hoc sign with `codesign --force --deep --sign - dist/AnchorCore.app` (free; notarization = $99/yr,
  deferred). Recipients right-click → Open once.
- Smoke-test a build with: `ANCHOR_PORT=8123 ANCHOR_OPEN_BROWSER=0 ANCHOR_DATA_DIR=/tmp/ac-smoke
  dist/AnchorCore.app/Contents/MacOS/AnchorCore` then curl `/health` (kills cleanly with pkill).

## Contract-first feature workflow (match existing conventions)

1. Schema: `models.py` + new Alembic migration in `backend/alembic/versions/` (down_revision = current head).
2. API: schemas in `schemas.py` + router in `backend/app/routers/` (register in `main.py`).
3. Wire into `AnswerEngine`/`pipeline` if it affects retrieval/ingestion.
4. Frontend: `types.ts` + `api.ts` + tab component.
5. Tests: new file `backend/tests/test_*.py` following existing style; run `pytest tests -q`.
6. Run `npm run build` for UI changes. Sync docs in the same commit (product-plan `— DONE` marker).
7. Bump version + tag only when releasing.

## Gotchas (learned the hard way — don't reintroduce)

- **`PRAGMA foreign_keys=ON` must stay** in `db.py` — ORM deletes rely on it (NOT NULL FK bug: entities
  used to get `item_id` NULLed on source delete).
- **Validate-before-write**: a `db.flush()` before a 422 validation error opened a write tx that stalled
  the whole test suite 15–140s (write-lock contention with the background scheduler). Project CRUD in
  `routers/projects.py` is the good pattern: validate sources first, then write.
- **SQLite single-writer flake**: the scheduler auto-syncs sources (including failing ones) in the
  background; a test's `start_and_wait` job can collide and die with `sqlite3.OperationalError: database
  is locked` ("job did not finish within 30s"). Intermittent, order/timing dependent — rerun the suite if
  a job dies this way; don't treat it as your feature being broken. The full suite is reliably green
  (~18s, 88 tests).
- **The suite runs without Ollama** — all E2E tests must pass degraded (rule-based classifier,
  keyword-only retrieval). Real-model probes are manual.
- **Shared test DB across the whole run** (conftest sets ONE SQLite file at module import; it accumulates
  across every test file) — scope assertions to your own source/entity ids and assert deltas, NEVER global
  emptiness (e.g. `test_onboarding_state` asserts `sources_count` grew, not that it equals 0).
- **`observer.unschedule()` takes a watch object, not a handler** (folder_watcher).
- **CI macOS build must use Homebrew Python** — python.org/toolcache CPython compiles sqlite3 without
  loadable extensions → sqlite_vec load crashes at startup. Don't revert `release.yml`.
- sqlite-vec native lib collected via `packaging.spec` (`*.dll`/`*.dylib`).
- Secrets: never log/return raw tokens — `GET /sources/{id}/config` masks `token`/`api_key`/`password`
  as `***set***`; `RedactingFormatter` strips secrets from logs.
- **Retrieval is index-backed (B33):** `vec_chunks` (sqlite-vec vec0, cosine) is created by migration
  `b33a0c1` and kept in sync by triggers; `AnswerEngine._vector_search` queries it (`k = :limit`)
  and falls back to the pure-Python cosine scan when the extension/table is unavailable or the
  embedding dims don't match. `/system/status.retrieval` exposes latency + backend (vec0 vs scan).
- **Single version source:** `__version__` in `app/config.py` (B34). Release = bump it → tag → push.
- **Job concurrency is bounded (B34):** `JobManager.MAX_CONCURRENT` (2) pipeline runs at once; excess
  syncs persist as `pending` and queue. `_age_decay` takes a fixed `now` per query.
- **PII gate (B39):** `sources.label` (internal|public|sensitive|pii) + provider trust — local
  (localhost) providers always trusted; cloud providers need `ANCHOR_CLOUD_TRUST=1`. Gated sources
  classify with rules (never sent to cloud) and skip cloud embedding, recording a `system_event`.
- **PII config + review (B30 partial):** `app/pii.py` = PII knowledge base (categories with field names +
  regexes + custom filter words) + `scan_text()` (local, no LLM). Chunks get `is_pii`/`pii_categories`
  auto-set on ingest (`pipeline._flag_pii`). API under `/pii` (config get/put, review list, decide, scan);
  UI = `PiiTab`. Config persists in `app_settings` keys `pii_custom_words` / `pii_disabled_categories`.
- **Answer gate (B30):** `AnswerEngine.ask` excludes sensitive/pii sources AND PII-flagged chunks from an
  unconfirmed cloud answer provider (`cloud_answer_trusted`, `ANCHOR_CLOUD_TRUST=1`), records a
  `system_event`, and refuses when nothing survives; follow-up rewrite is skipped when untrusted.
  `/qa/public` answers ONLY from `public` sources (share/MCP-safe surface; AskTab Public-only toggle).
  PII-flagged chunks are also skipped from cloud embedding in `pipeline._embed_item`.
- **SecretStore fallback is a per-key file store** (`secrets.enc.<sha256(key)>` files) — the old
  single-blob design made `delete(key)` wipe *every* stored secret. `store_source_config` treats a
  `'***set***'` placeholder or absent field as "leave the stored secret unchanged", and an explicit
  empty string as "clear it". Don't reintroduce a single-file fallback.
- **`POST /sources` for a Jira connector does NOT validate the instance** — config-only; only a sync
  fails if credentials are wrong. Fine for tests/onboarding.
- **Frontend build type-checks** (`tsc -b && vite build`) — unused vars/imports fail the build (see the
  `ollamaOk` lesson). Keep TS types in `types.ts` in sync with backend schemas or the build breaks.

## UI notes

- `frontend/src/App.tsx` — tabs + header (project picker scopes Ask via B15); mounts `OnboardingWizard`
  when `/system/onboarding` reports `needs_wizard` (first launch, no sources) and it isn't dismissed
  (`localStorage['wizard-dismissed']`).
- `frontend/src/OnboardingWizard.tsx` — B8: checks (Ollama install/pull per platform) → connect a folder
  or the bundled `sample/` corpus (sync with live progress) → ask a question with cited answer.
  Sample path resolves via `Path(__file__).resolve().parents[3] / "sample"` (dev checkout only; frozen
  apps report `sample.available: false`).
- `AskTab` renders turns newest-first (B19). `SourcesTab` is a thin composition (B37): project/source
  logic lives in `tabs/sources/{ProjectSection,SourceForm,SourceRow}.tsx`; job polling is the single
  `useJobsPoll` hook (TanStack Query); `projects` state lives in `ProjectsContext` (no duplicate fetch).
- `theme.ts` holds the shared color tokens (B37) — don't inline new hex literals; extend the token set.
- `api.ts` `request()` throws `Error(detail)` from the response JSON; it accepts an `AbortSignal` +
  applies a 30s timeout, surfacing aborts as `RequestAbortedError` ("Request cancelled"); tabs surface
  `error` state.
- `main.tsx` wraps the app + each tab in an `ErrorBoundary` (B36) — a render throw shows a recoverable
  card with "Copy error / Download log" instead of a white screen in `console=False` packaged builds.
- `types.ts` mirrors backend schemas; keep in sync when adding API surface.
