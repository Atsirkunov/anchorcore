# AnchorCore — agent guide (living file)

Read this before doing anything. It is the distilled backend/architecture logic so
a fresh session doesn't have to re-explore the whole repo. Update it whenever you
learn something that would have helped at the start of a session.

## What this is

Local-first "company memory": connect sources (folder, Jira, Linear) → classify into
entities with provenance → hybrid RAG Q&A with citations. React SPA served by one
Axum process. Stack: **Rust (Axum) + SQLite (sqlite-vec + FTS5)** · React/Vite · Ollama local + BYO cloud OpenAI-compatible. Packaged as single binary (`frontend/dist` embedded). **Python `backend/` is deprecated** — legacy conformance + `hosting/` Postgres only (see `rust/README.md` + `rust/BACKLOG.md` + `docs/rust-port.md`). Release cadence: bump `rust/Cargo.toml:6` → `python scripts/sync_version.py` → tag `vX.Y.Z` → push (CI builds `AnchorCore-rust-*`).

Repo: `git@github.com:Atsirkunov/anchorcore.git`. Backend under `backend/` (deprecated), UI under `frontend/`, Rust under `rust/` (shipped 1.0.11, source of truth).

## How to run / test / release (Rust is shipped, Python deprecated as of 1.0.11)

- Dev (Rust — shipped): `cargo run -p anchorcore -- --port 8000 --data-dir ~/.anchorcore` (migrations, watcher, jobs; `cargo test -p anchorcore` 47, `cargo check 0`). For conformance vs Python: `cargo run -p anchorcore -- --port 8123 --data-dir /tmp/ac-dev` + `PYTHONPATH=backend ANCHOR_TEST_RUST_URL=http://127.0.0.1:8123 pytest -q --ignore=test_mcp.py`
- Dev (Python legacy — deprecated): `./start.sh` (venv, migrations, Ollama, backend :8000) — conformance only + `hosting/` Postgres. Do not use for new features.
- Backend tests (conformance): `PYTHONPATH=backend ANCHOR_TEST_RUST_URL=http://127.0.0.1:8123 pytest -q --ignore=test_mcp.py` (`110/17` honest, `127` collect, `skipIf` for Python-internal, `rust/BACKLOG.md` R9.2) via CI `rust-conformance`. `cargo test -p anchorcore` 47 + `cargo check 0` green.
- Frontend: `cd frontend && npm run build` (tsc + vite; must pass before a UI change is done) — embedded via `rust/crates/anchorcore/src/frontend.rs:1`
- Release (single source): edit `rust/Cargo.toml:6` `workspace.package.version` → `python scripts/sync_version.py` (writes `backend/app/config.py:11`) → `cargo test` + `cargo build --release` (`frontend/dist` embedded, `9.8M`, `codesign valid`) → `git tag vX.Y.Z` → `git push origin vX.Y.Z` (CI builds `AnchorCore-rust-*`)
- **The Rust binary embeds `frontend/dist` at build time** — after any UI change you must `cargo build --release` (or `npm run build` + `cargo build`), or the exe ships stale UI. `hosting/` stays Python per R10.7.

## Backend architecture map (Rust is source, `backend/` is legacy mirror)

- `rust/crates/anchorcore/src/main.rs` — wires everything: `secrets`, `SettingsService`, `Classifier`, `Embedder`, `Pipeline`, `AnswerEngine`, `Scheduler`, `JobManager`; runs migrations; serves bundled UI last.
- `rust/crates/anchorcore/src/db.rs` — `open_db`/`init_db`, `PRAGMA foreign_keys=ON`, `WAL`, migrations via `rusqlite_migration`; same SQLite file as Python.
- `rust/crates/anchorcore/src/sources.rs` / `entities.rs` / `jobs.rs` / `answer.rs` / `pipeline.rs` — mirror `backend/app/routers/*` + `answer_engine.py`/`pipeline.py`. Keep `spawn_blocking` + per-key `secrets.enc.<sha256>`.
- `backend/app/main.py` (deprecated) — legacy FastAPI wiring, same logic as Rust `main.rs`. Use only for `hosting/` + conformance.
- `backend/app/models.py` (deprecated) — SQLAlchemy models: `Source` (`label` B39), `IngestedItem`, `Entity` (`window_text`/`window_index` B26, `dispute_count` B3), `Relationship`, `Chunk` (`kind`: document|entity|distilled B18, `is_pii`/`pii_categories` B30; Phase 14 adds `section_id`/`level`/`path` + `kind=section_summary|doc_summary`), `Job`, `Project` + `project_sources`, `SystemEvent`, `AppSetting`, `MergeAction`, `User` (B40 hosted auth). Phase 14 adds `Section` (parent/level/path/summary+embedding) + `Tag`/`ChunkTag` (dynamic taxonomy, reuse if `cosine>0.82`). `content_hash()` helper (re-exported via `hashing.py`).
- `backend/app/schemas.py` — Pydantic request/response models. `SourceOut` does NOT include `config`
  (config is fetched via `GET /sources/{id}/config` which masks secrets); `SourceCreate/Update` validate `label` (B39).
- `backend/app/answer_engine.py` — **the core retrieval pipeline** (see below). `vec_chunks` vec0 index (B33) with Python-scan fallback; gates `sensitive`/`pii` + `is_pii` chunks from cloud answer when `ANCHOR_CLOUD_TRUST` missing (B30).
- `backend/app/pipeline.py` — ingestion orchestration: classify (windowed, hash-skipped, concurrency-limited),
  distill (B18), embed (IDF-gated, `is_pii` chunks skip cloud embed B30, `_flag_pii`), PII gate (B39). Chunking split to `chunking.py`/`hashing.py`/`distill.py` (B35). **Scale gap (Phase 14):** `chunk_document` derives `sections` then discards the tree — no `section_id`/`path` persisted; retrieval is flat RRF over `vec_chunks`+FTS5 (no hierarchical pruning). Planned `sections` + `tags`/`chunk_tags` persisted tree + summary nodes → coarse-to-fine fetch.
- `backend/app/chunking.py` / `hashing.py` / `distill.py` — extracted from `pipeline.py` (B35). `chunking.py:14` `chunk_text` (800/100 char-based), `chunking.py:49` `chunk_document` (heading-aware `_is_heading` `§434`/`4.2.1`/`Article 12`/ALL-CAPS → `1600` max paragraph-packed sections), `chunking.py:99` `classify_windows` (16000, 50% overlap). Rust parity `rust/crates/anchorcore/src/chunking.rs:16/68/133`. Cleaning before chunking `cleaning.py:17` (`clean_text` + `repeated_lines`).
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

Current access is **flat** — RRF over `vec_chunks` vec0 (`k=top_k*4` `answer_engine.py:639`) + FTS5 bm25 `LIMIT top_k*4` (`answer_engine.py:710`), scored `Σ w/(60+rank)` `retrieval.rs:634`, age-decayed, per-item diversity-capped, `expand_context ±1` `answer_engine.py:377`, deduped by `_content_signature`. `retrieval.rs:192` already parametriz-es `IN` for `source_ids` — extend same pattern for `section_id` filtering in Phase 14. Fallback `vector_search_scan` `retrieval.rs:240` full-scans `chunks WHERE embedding IS NOT NULL` (must be avoided at 150k scale via TOC pruning).

**Phase 14 access (planned):** same pipeline with TOC pre-stage: (1) SQL/tag filter on `sections.path`/`chunk_tags` + `project source_ids`, (2) summary-node vector search (~5k sections vs 150k leaves, `kind=section_summary` or `sections.summary_embedding`), (3) leaf vec0 `WHERE c.section_id IN (:top_sections)`. Persists `chunk_document` `sections` tree instead of discarding it.

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

## Packaging (Rust binary — shipped, PyInstaller is legacy)

- **Rust:** `cargo build --release -p anchorcore` embeds `frontend/dist` via `src/frontend.rs:1` (`include_dir!`), yields `rust/target/release/anchorcore` (`9.8M`+`3.4M`, `codesign valid`). `dist/AnchorCore-rust-macos.zip` / `windows.zip` are the ship artifacts; `release.yml` builds on every `v*` tag. Ad-hoc sign `codesign --force --deep --sign -`.
- **Legacy PyInstaller (hosting only):** `packaging.spec` `onedir + console=False` → `dist/AnchorCore.app` / `dist/AnchorCore/` — keep for `hosting/` reference, not for local. Do not add `backend/` features.
- **Legacy `backend/run_app.py`** frozen entry: `_guard_windowed_stdio()` devnull — keep for `hosting` smoke only.
- macOS CI: Rust uses `sqlite-vec` statically linked (no Homebrew Python needed for local); `hosting` Python still needs Homebrew for `vec0` if used.
- Smoke-test Rust: `ANCHOR_OPEN_BROWSER=0 ANCHOR_DATA_DIR=/tmp/ac-smoke rust/target/release/anchorcore --port 8123` then `curl /health`.

## Contract-first feature workflow (Rust-only, `backend/` frozen)

0. Design for cyclomatic complexity: **every new/changed function must stay ≤ 15 CCN** (lizard) — split orchestrators into named phase helpers, use guard clauses (`let Some(x) = y else { return ... }`) instead of nested `if let`/`match`, and reuse `with_db`/`run_db` (`pipeline.rs`) for any `spawn_blocking` DB work. Gate: `lizard -l rust -w rust/crates/anchorcore/src -C 15` — no new warnings beyond the accepted `is_public_path` (auth.rs decision table, do-not-touch). See `rust/AGENTS.md` "Cyclomatic complexity" + `rust/BACKLOG.md` Phase 13.
1. Schema: `rust/crates/anchorcore/src/db.rs` + new migration in `rust/crates/anchorcore/migrations/*.sql` (add to `db.rs:migrations()` list; keep `PRAGMA foreign_keys=ON`). Mirror in `backend/alembic` only if `hosting/` needs it.
2. API: handler in `rust/crates/anchorcore/src/*.rs` + route in `main.rs` (register last, `frontend` fallback stays last).
3. Wire into `retrieval.rs`/`pipeline.rs` if it affects retrieval/ingestion.
4. Frontend: `types.ts` + `api.ts` + tab component; keep `theme.ts` tokens, don't inline hex.
5. Tests: `cargo test -p anchorcore` (47) + `cargo check 0`; conformance `PYTHONPATH=backend ANCHOR_TEST_RUST_URL=http://127.0.0.1:8123 pytest -q --ignore=backend/tests/test_mcp.py` (`110/17`). Add `backend/tests` only for `hosting` parity.
6. Run `npm run build` for UI changes (must re-`cargo build --release` to embed). Sync docs in same commit (`product-plan` `— DONE`).
7. Bump `rust/Cargo.toml:6` → `python scripts/sync_version.py` → tag only when releasing.

## Rust port — shipped as of `v1.0.10` `c37202e` (Python retired, `R11.1` wired)

* Now **shipped artifact**: `rust/` workspace is source of truth (`rust/Cargo.toml:6` `1.0.10` single source via `scripts/sync_version.py`), same API + same SQLite file as Python. `backend/` is legacy conformance + hosted Postgres (R10.7 `hosting/` stays Python). `cargo test 47` + `cargo check 0` + `110/17` honest vs Rust (`127` collect) via `ci.yml:12` `rust-conformance`.
* `cargo run -p anchorcore -- --port 8123` on `:8123` vs `PYTHONPATH=backend pytest` conformance (`110/17` honest, was `115/8/3`/`124/3`), `cargo test 47` + `cargo check 0` (R11.3 `RUSTFLAGS="-D warnings"` + `clippy -- -D unwrap_used` with local `allow`), watcher `watcher/service.rs:12` `3s` debounce + `scheduler.rs:30` `reload_sources` `interval.tick → jobs.create_job` + `Pipeline` executor (`R11.1`), `db.rs:22` `open_db` fast path, `jobs.rs:101` `PROMOTE_LOCK` + `pipeline.rs:61` wait-loop, `answer.rs:78` `rewrite_followup` + `generate_answer` `Conversation so far` (`R11.4` only pronoun, `R11.7` `6` cap), `auth.rs:21` `429` `5/60s` per-email (`R11.5` `ANCHOR_TRUSTED_PROXY`).
* **Post-1.0.10 polish:** `rust/BACKLOG.md:127` Phase 11 `R11.1–R11.10` (P0 `R11.1` wiring + `R11.2` no degraded persist `stable_hash` FNV, P1 `R11.3` lints + `R11.4` heuristic + `R11.5` XFF, P2 `R11.6` score normalize + `R11.8` `OnceLock` cache + `R11.9` real `JobManager`). Full write-up: `docs/handover-2026-08-22-v1.0.10.md:1` + `docs/review-2026-08-21-full.md:1`.
* AI agents: pick one task from `rust/BACKLOG.md` **Phase 11** (`R11.1`→`R11.2`→`R11.3`→`R11.4/5`→`R11.6-10`); keep `PRAGMA foreign_keys=ON` + `spawn_blocking` + per-key `secrets.enc.<sha256>` 64 hex.

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
- **Single version source:** `rust/Cargo.toml:6` `workspace.package.version` (`1.0.10` via `scripts/sync_version.py` → `backend/app/config.py:11`, see `rust/BACKLOG.md:63` R5.3; `ci.yml:12` `version-drift` checks both `scripts/sync_version.py` + `rust/scripts/sync_version.py` — they are identical, duplicate kept for `cargo run` from `rust/` vs root, drift risk noted `rust/BACKLOG.md:127` R11.10; `rust-conformance` job only gates merges when branch protection is enabled).
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
