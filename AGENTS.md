# AnchorCore — agent guide (living file)

> Python backend archived: `backend/` is not in the tree — restore it first with `git checkout archive/python-final -- backend` before running any `backend/` command below.

Read this before doing anything. It is the distilled backend/architecture logic so
a fresh session doesn't have to re-explore the whole repo. Update it whenever you
learn something that would have helped at the start of a session.

## What this is

Local-first "company memory": connect sources (folder, Jira, Linear) → classify into
entities with provenance → hybrid RAG Q&A with citations. React SPA served by one
Axum process. Stack: **Rust (Axum) + SQLite (sqlite-vec + FTS5)** · React/Vite · Ollama local + BYO cloud OpenAI-compatible. Packaged as single binary (`frontend/dist` embedded). **Python `backend/` is deprecated** — legacy conformance only; `hosting/` is the one-container Team image (R19.1) (see `rust/README.md` + `rust/BACKLOG.md` + `docs/rust-port.md`). Release cadence: bump `rust/Cargo.toml:6` → `python scripts/sync_version.py` → draft `docs/release-notes/vX.Y.Z.md` (Release-notes rule) → tag `vX.Y.Z` → push (CI builds `AnchorCore-{windows,macos,linux}.zip`).

Repo: `git@github.com:Atsirkunov/anchorcore.git`. Backend under `backend/` (deprecated), UI under `frontend/`, Rust under `rust/` (shipped 1.0.20, source of truth).

## How to run / test / release (Rust is shipped, Python deprecated as of 1.0.12)

- Dev (Rust — shipped): `cargo run -p anchorcore -- --port 8000 --data-dir ~/.anchorcore` (migrations, watcher, jobs; `cargo test -p anchorcore` 59, `cargo check 0`). For conformance vs Python (restore `backend/` from tag `archive/python-final` first): `cargo run -p anchorcore -- --port 8123 --data-dir /tmp/ac-dev` + `PYTHONPATH=backend ANCHOR_TEST_RUST_URL=http://127.0.0.1:8123 pytest -q --ignore=test_mcp.py`
- Dev (Python legacy — deprecated): `./start.sh` (venv, migrations, Ollama, backend :8000) — conformance only. Do not use for new features.
- Backend tests (conformance — restore `backend/` from the archive tag first): `PYTHONPATH=backend ANCHOR_TEST_RUST_URL=http://127.0.0.1:8123 pytest -q --ignore=test_mcp.py` (honest accounting, `skipIf` for Python-internal, `rust/BACKLOG-archive.md` R9.2) via CI `rust-conformance`. `cargo test -p anchorcore` green + `cargo check` clean.
- Frontend: `cd frontend && npm run build` (tsc + vite; must pass before a UI change is done) — embedded via `rust/crates/anchorcore/src/frontend.rs:1`
- Team image (one container, R19.1/B59): `docker compose -f hosting/docker-compose.yml up --build` (or `docker build -f hosting/Dockerfile -t anchorcore:team .`). License gate `ANCHOR_EDITION=team` → `rust/crates/anchorcore/src/license.rs` (issue: `python scripts/make_license.py`); CI smoke `docker-team`; GHCR publish on tags.
- Release (single source): edit `rust/Cargo.toml:6` `workspace.package.version` → `python scripts/sync_version.py --check` (Cargo-only since the archive) → `cargo test` + `cargo build --release` (`frontend/dist` embedded, `9.8M`, `codesign valid`) → draft `docs/release-notes/vX.Y.Z.md` (Release-notes rule) → `git tag vX.Y.Z` → `git push origin vX.Y.Z` (CI builds `AnchorCore-{windows,macos,linux}.zip`)
- **The Rust binary embeds `frontend/dist` at build time** — after any UI change you must `cargo build --release` (or `npm run build` + `cargo build`), or the exe ships stale UI. `hosting/` runs this same Rust binary in one container (R19.1/B59).
- **Release notes (rule): every `v*` tag ships human-readable notes — no empty GitHub Release body.** Before tagging, draft `docs/release-notes/vX.Y.Z.md` from the commits since the previous tag + the shipped log + handovers, written for users (and their agents), not maintainers: `Highlights` (what's new, plain verbs), `Fixed` (user-visible bugs only), `Upgrade notes` (migrations, behavior changes, anything to re-run) — translate R-codes/B-codes into plain language, never paste internal IDs unexplained. Commit the file with the release, and after CI publishes the release attach it as the body (`gh release edit vX.Y.Z --notes-file docs/release-notes/vX.Y.Z.md`). A release without notes is incomplete — backfill before announcing.

## Backend architecture map (Rust is source, `backend/` is legacy mirror)

- `rust/crates/anchorcore/src/main.rs` — wires everything: `secrets`, `SettingsService`, `Classifier`, `Embedder`, `Pipeline`, `AnswerEngine`, `Scheduler`, `JobManager`; runs migrations; serves bundled UI last.
- `rust/crates/anchorcore/src/db.rs` — `open_db`/`init_db`, `PRAGMA foreign_keys=ON`, `WAL`, migrations via `rusqlite_migration`; same SQLite file as Python.
- `rust/crates/anchorcore/src/sources.rs` / `entities.rs` / `jobs.rs` / `answer.rs` / `pipeline.rs` — mirror `backend/app/routers/*` + `answer_engine.py`/`pipeline.py`. Keep `spawn_blocking` + per-key `secrets.enc.<sha256>`.
- `rust/crates/anchorcore/src/backup.rs` — Phase 25 backup/export: uid-based `.anchorcore-backup` archives (`POST /backup/export|import`), Argon2id + XChaCha20-Poly1305 secrets bundle, dry-run report (apply-then-rollback), replace + safety backup, `jobs.kind='embed'` re-embed. CLI: `src/cli.rs` (`anchorcore export|import`; prefers a running API). UI: `frontend/src/BackupCard.tsx` (System tab). Docs: `docs/backup.md`.
- `backend/app/main.py` (archived on tag `archive/python-final`) — legacy FastAPI wiring, same logic as Rust `main.rs`. On the archive tag — restore it to read any `backend/` path below.
- `backend/app/models.py` (deprecated) — SQLAlchemy models: `Source` (`label` B39), `IngestedItem`, `Entity` (`window_text`/`window_index` B26, `dispute_count` B3), `Relationship`, `Chunk` (`kind`: document|entity|distilled B18, `is_pii`/`pii_categories` B30; Phase 14 adds `section_id`/`level`/`path` + `kind=section_summary|doc_summary`), `Job`, `Project` + `project_sources`, `SystemEvent`, `AppSetting`, `MergeAction`, `User` (B40 hosted auth). Phase 14 adds `Section` (parent/level/path/summary+embedding) + `Tag`/`ChunkTag` (dynamic taxonomy, reuse if `cosine>0.82`). `content_hash()` helper (re-exported via `hashing.py`).
- `backend/app/schemas.py` — Pydantic request/response models. `SourceOut` does NOT include `config`
  (config is fetched via `GET /sources/{id}/config` which masks secrets); `SourceCreate/Update` validate `label` (B39).
- `backend/app/answer_engine.py` — **the core retrieval pipeline** (see below). `vec_chunks` vec0 index (B33) with Python-scan fallback; gates `sensitive`/`pii` + `is_pii` chunks from cloud answer when `ANCHOR_CLOUD_TRUST` missing (B30).
 - `backend/app/pipeline.py` — ingestion orchestration: classify (windowed, hash-skipped, concurrency-limited),
  distill (B18), embed (IDF-gated, `is_pii` chunks skip cloud embed B30, `_flag_pii`), PII gate (B39). Chunking split to `chunking.py`/`hashing.py`/`distill.py` (B35). **Phase 14 (1.0.12):** `chunk_document_with_sections` persists `sections` tree (`parent/level/path/summary`) + `tags`/`chunk_tags` taxonomy (`cosine>0.82` reuse) — retrieval is coarse-to-fine via `toc_search` (see `docs/guides/hierarchical-toc.md`).
- `backend/app/chunking.py` / `hashing.py` / `distill.py` — extracted from `pipeline.py` (B35). `chunking.py:14` `chunk_text` (800/100 char-based), `chunking.py:49` `chunk_document` (heading-aware `_is_heading` `§434`/`4.2.1`/`Article 12`/ALL-CAPS → `1600` max paragraph-packed sections), `chunking.py:99` `classify_windows` (16000, 50% overlap). Rust parity `rust/crates/anchorcore/src/chunking.rs:16/68/133`. Cleaning before chunking `cleaning.py:17` (`clean_text` + `repeated_lines`).
- `backend/app/pii.py` — PII knowledge base + `scan_text` (B30), categories + custom words, persisted in `app_settings`.
- `backend/app/auth.py` — hosted auth (B40): PBKDF2 + HS256 JWT, `get_current_user`/`require_auth` (off when `ANCHOR_AUTH_SECRET` empty).
- `backend/app/classifier.py` / Rust `classifier.rs` — per-doc-type extraction prompts
  (`standards|runbook|documentation|meeting|decision_log|prd|code|general`), `detect_document_type` (extension fast path for source files, else LLM with
  `detect_rules` fallback), `classify`, `distill`; rule-based fallbacks for all (tests must pass without
  Ollama) — note-only types (standards/runbook/documentation) never emit decision/action. Takes
  `cloud_trusted` (B39). **R22.1:** the Rust pipeline must call `Classifier::classify`/`distill` per
  window — don't route extraction back through `classify_rules` (that shipped 1.0.10–1.0.14 and fabricated
  decisions/actions from process docs). **R33.2:** `code` skips extraction entirely (code is code — the code itself is the retrievable unit, no LLM call, legacy code entities purged on re-sync); entities/citations/search hits still carry the parent item's `doc_type` so code provenance stays visibly badged (`code`) in Entities/Review/Ask + MCP.
- `backend/app/routers/` — `sources.py` (create/list/update/delete + config + jobs, label B39),
  `entities.py` (`GET /{id}`, `PATCH`, `GET /{id}/related`, `POST /{id}/dispute`, `GET /{id}/disputes`, **`GET /{id}/context` B27 neighbour expansion**; `review_router` for low-confidence/duplicates/merge), `qa.py` (`POST /qa` + `POST /qa/public` B30 share-safe), `projects.py`, `settings.py`, `system.py` (status/errors/logs/onboarding B8), `pii.py` (`/pii/config`, `/pii/review`, `/pii/review/{id}`, `/pii/scan/{source}`), `auth.py` (`/auth/status`, `/auth/signup`, `/auth/login`, `/auth/me` B40).
- `backend/app/secrets.py` — `SecretStore` (OS keychain, encrypted-file fallback when
  `ANCHOR_SECRETS_NO_KEYRING=1`); `SECRET_SOURCE_FIELDS` single source of truth (B34); `store_source_config`/`resolve_source_config` handle secret fields. Never ship secrets to the UI.
- `backend/app/app_settings.py` — `SettingsService`: env `.env` is default, DB `app_settings` overrides
  win, secrets resolve from SecretStore. Read effective values at call time (no restart). `SETTING_KEYS`/
  `SECRET_KEYS`/`ALL_KEYS` control what the Settings UI exposes.
- `backend/app/config.py` — single source `__version__` (B34) + pydantic `Settings` (env prefix `ANCHOR_`, `.env` file). Adds `auth_secret`/`auth_token_hours`/`auth_enabled` (B40), `cloud_trust` helpers (B30/B39), retrieval knobs, `embed_dim` (B33 vec0).
- `backend/app/db.py` — engine/session; **sets `PRAGMA foreign_keys=ON` (critical, deletions depend on it)**. `engine` is env-driven (`resolved_database_url` → SQLite local, `postgresql+psycopg` was the old hosted path — `hosting/` is Rust single-container now).
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

Current access is **hierarchical (1.0.12)** — coarse-to-fine via `toc_search` `retrieval.rs:836`: (1) tag/SQL prune (`matching_tag_sections` `chunk_tags`/`sections.path` + `project source_ids`), (2) summary-node vector search `c.kind IN (section_summary,doc_summary)` `~5k not 150k` (`summary_vector_search_filtered` `retrieval.rs:548`), (3) leaf vec0 `WHERE c.section_id IN (:top5)` + FTS `AND c.section_id IN` (`leaf_vector_search_filtered` `retrieval.rs:704`), RRF-fused. Falls back to flat RRF over `vec_chunks` vec0 (`k=top_k*4` `answer_engine.py:639`) + FTS5 `LIMIT top_k*4` (`answer_engine.py:710`) `retrieval.rs:634` with age decay, diversity cap, `expand_by_section` (`WHERE section_id = ?`) vs `±1`, deduped. `vector_search_scan` `retrieval.rs:276` now filtered by `section_id` + entity bypass, avoided at scale.

**Phase 14 details:** `docs/guides/hierarchical-toc.md` — `GET /sections?item_id=&source_id=` + `GET /tags` + `Breadcrumb` `Art 12 › 12.3` + `tag_reuse_threshold` `settings.rs:7` (`0.82` env `ANCHOR_TAG_REUSE_THRESHOLD` → `app_settings`).

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

- **Rust:** `cargo build --release -p anchorcore` embeds `frontend/dist` via `src/frontend.rs:1` (`include_dir!`), yields `rust/target/release/anchorcore` (`9.8M`+`3.4M`, `codesign valid`). `dist/AnchorCore-macos.zip` / `AnchorCore-windows.zip` are the ship artifacts; `release.yml` builds on every `v*` tag. Ad-hoc sign `codesign --force --deep --sign -`.
- **Legacy PyInstaller (archived):** `packaging.spec` (on the tag) `onedir + console=False` → `dist/AnchorCore.app` / `dist/AnchorCore/` — historical reference only, not for local. Do not add `backend/` features.
- **Legacy `backend/run_app.py`** (on the archive tag) frozen entry: `_guard_windowed_stdio()` devnull — historical reference only (R19.1: hosting is the Rust image now).
- macOS CI: Rust needs no Homebrew Python (sqlite is bundled via `rusqlite`); `hosting/` runs this same binary in one container.
- Smoke-test Rust: `ANCHOR_OPEN_BROWSER=0 ANCHOR_DATA_DIR=/tmp/ac-smoke rust/target/release/anchorcore --port 8123` then `curl /health`.

## Contract-first feature workflow (Rust-only, `backend/` frozen)

0. Design for cyclomatic complexity: **every new/changed function must stay ≤ 15 CCN** (lizard) — split orchestrators into named phase helpers, use guard clauses (`let Some(x) = y else { return ... }`) instead of nested `if let`/`match`, and reuse `with_db`/`run_db` (`pipeline.rs`) for any `spawn_blocking` DB work. Gate: `lizard -l rust -w rust/crates/anchorcore/src -C 15` — no new warnings beyond the accepted `is_public_path` (auth.rs decision table, do-not-touch). See `rust/AGENTS.md` "Cyclomatic complexity" + `rust/BACKLOG.md` Phase 13.
1. Schema: `rust/crates/anchorcore/src/db.rs` + new migration in `rust/crates/anchorcore/migrations/*.sql` (add to `db.rs:migrations()` list; keep `PRAGMA foreign_keys=ON`). Mirror in `backend/alembic` only for conformance parity.
2. API: handler in `rust/crates/anchorcore/src/*.rs` + route in `main.rs` (register last, `frontend` fallback stays last).
3. Wire into `retrieval.rs`/`pipeline.rs` if it affects retrieval/ingestion.
4. Frontend: `types.ts` + `api.ts` + tab component; keep `theme.ts` tokens, don't inline hex.
5. Tests: `cargo test -p anchorcore` green + `cargo check` clean; conformance (restore `backend/` from tag first) `PYTHONPATH=backend ANCHOR_TEST_RUST_URL=http://127.0.0.1:8123 pytest -q --ignore=backend/tests/test_mcp.py` (green). Add `backend/tests` only for conformance parity.
6. Run `npm run build` for UI changes (must re-`cargo build --release` to embed). Sync docs in same commit (`product-plan` `— DONE`).
7. Bump `rust/Cargo.toml:6` → `python scripts/sync_version.py` → tag only when releasing.

## Rust port — shipped as of `v1.0.20` (Phase 14 TOC + R12.5 MCP)

* Now **shipped artifact**: `rust/` workspace is source of truth (`rust/Cargo.toml:6` `1.0.20` single source via `scripts/sync_version.py`), same API + same SQLite file as Python. `backend/` is legacy conformance only (R19.1: `hosting/` is the one-container Rust Team image). `cargo test` + `cargo check` green + honest conformance vs Rust (`127` collect) via `ci.yml:12` `rust-conformance`.
* `cargo run -p anchorcore -- --port 8123` on `:8123` vs `PYTHONPATH=backend pytest` conformance (`110/17` honest, was `115/8/3`/`124/3`), `cargo test` + `cargo check` clean (R11.3 `RUSTFLAGS="-D warnings"` + `clippy -- -D unwrap_used` with local `allow`), watcher `watcher/service.rs:12` `3s` debounce + `scheduler.rs:30` `reload_sources` `interval.tick → jobs.create_job` + `Pipeline` executor (`R11.1`), `db.rs:22` `open_db` fast path, `jobs.rs:101` `PROMOTE_LOCK` + `pipeline.rs:61` wait-loop, `answer.rs:78` `rewrite_followup` + `generate_answer` `Conversation so far` (`R11.4` only pronoun, `R11.7` `6` cap), `auth.rs:21` `429` `5/60s` per-email (`R11.5` `ANCHOR_TRUSTED_PROXY`), Phase 14 `chunking.rs:68` `chunk_document_with_sections` `sections` + `tags.rs:50` `0.82` + `retrieval.rs:836` `toc_search` `migrations/11_12` + `sections.rs`/`tags.rs` `GET /sections|/tags` + `R12.5` `mcp.rs:92` `public_only` `audit_mcp`.
* **Post-1.0.10 polish (archived):** Phase 11 `R11.1–R11.10` + Phase 12 `R12.5` MCP `public_only` audit + Phase 14 `R14.1–R14.6` hierarchical TOC in `rust/BACKLOG-archive.md` (see `docs/guides/hierarchical-toc.md`). Full write-up: `docs/handover-2026-08-28-v1.0.11.md:1` + `docs/review-2026-08-21-full.md:1`.
* AI agents: pick open work from `rust/BACKLOG.md` — **Phase 20** (`R20.*`, MCP zero-setup UX — `docs/plan-mcp-zero-setup.md`, 2 left), **Phase 23** (`R23.*`, agent-friendly config, 4 left + 1 deferred) or **Phase 18** (`R18.*`, website audit, 1 partial); **Phase 25 (backup/export) is shipped** (R25.1–R25.7, `docs/backup.md`, Windows-verified — the macOS acceptance leg is still open). Shipped work is in its shipped log, completed Phases 0–17 in `rust/BACKLOG-archive.md`. Keep `PRAGMA foreign_keys=ON` + `spawn_blocking` + per-key `secrets.enc.<sha256>` 64 hex.

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
- **Single version source:** `rust/Cargo.toml:6` `workspace.package.version` (`1.0.20` via `scripts/sync_version.py` (backend leg removed after the archive), see `rust/BACKLOG-archive.md` R5.3; `ci.yml:12` `version-drift` checks both `scripts/sync_version.py` + `rust/scripts/sync_version.py` — they are identical, duplicate kept for `cargo run` from `rust/` vs root, drift risk noted `rust/BACKLOG-archive.md` R11.10; `rust-conformance` job only gates merges when branch protection is enabled).
- **Job concurrency is bounded (B34):** `JobManager.MAX_CONCURRENT` (2) pipeline runs at once; excess
  syncs persist as `pending` and queue. `_age_decay` takes a fixed `now` per query.
- **PII gate (B39):** `sources.label` (internal|public|sensitive|pii) + provider trust — local
  (localhost) providers always trusted; cloud providers need `ANCHOR_CLOUD_TRUST=1`. Gated sources
  classify with rules (never sent to cloud) and skip cloud embedding, recording a `system_event`.
- **PII is people-driven (R35.1):** one manual mark at whole-document level (`ingested_items.is_pii`,
  default unmarked; `PATCH /items/:id {"is_pii": bool}`; Entities toggle; `item_is_pii` on entity JSON;
  backup round-trips it). The B30 auto-detect stack is deleted, not deprecated: `pii.rs` (KB + `scan_text`),
  `/pii/*` routes, `PiiTab`, the ingest `flag_pii` phase, the `?gate=provider` honor system on
  entities/items reads, and the chunk-`is_pii` halves of the answer/embed gates. `chunks.is_pii`/
  `pii_categories` are still in the schema but unwritten (drop migration is R35.3); how manual marks
  gate cloud flows is R35.2 — until then marks are advisory + visible. Never reintroduce auto-flagging
  or a caller-asserted gate without an owner decision (see `rust/docs/decisions.md` R35.1).
- **Answer gate is label-only (B30 as cut by R35.1):** `ask`/`search` exclude sensitive/pii *sources* from an
  unconfirmed cloud answer provider (`cloud_answer_trusted`, `ANCHOR_CLOUD_TRUST=1`), records a
  `system_event`, and refuses when nothing survives; follow-up rewrite is skipped when untrusted.
  `/qa/public` answers ONLY from `public` sources (share/MCP-safe surface; AskTab Public-only toggle).
  Labeled sources also skip cloud classify (rules fallback) and cloud embedding.
- **SecretStore fallback is a per-key file store** (`secrets.enc.<sha256(key)>` files) — the old
  single-blob design made `delete(key)` wipe *every* stored secret. `store_source_config` treats a
  `'***set***'` placeholder or absent field as "leave the stored secret unchanged", and an explicit
  empty string as "clear it". Don't reintroduce a single-file fallback.
- **`POST /sources` for a Jira connector does NOT validate the instance** — config-only; only a sync
  fails if credentials are wrong. Fine for tests/onboarding.
- **Folder sync is incremental via stat fingerprints (migration 17):** `ingested_items.file_mtime_ms/file_size`
  decide *whether to extract*; the content hash stays the authority for *whether to reclassify*.
  `run_folder_sync` lists (stat-only) → purges from the listing → skips fingerprint matches; the watcher
  compares via the same `list_files` (never `fetch`). Job `result` carries live `items/unchanged` counts,
  so progress reads `200/514 · 3 changed` and the done line adds `, N unchanged`. A NULL fingerprint
  (other connectors, pre-migration rows, backup imports) always extracts once, then backfills.
- **Folder connector + purge (B57/R22.1):** files that exist but cannot be read are returned as
  `skipped` and must never be purged; PDFs extract via `pdf-extract` in Rust (was a stub until 2026-10-11 —
  sync used to delete ingested PDFs because fetch skipped them). Keep new file types' error paths
  reporting, not dropping.
- **File formats live in `extract.rs` (R24.2/R24.3, B9):** folder + Drive both call `extract_file`/`extract_bytes` — notes/docs, data/config and source code (most popular languages) are plain text via `TEXT_EXTS`/`TEXT_NAMES`; formats that need a real extractor are Word (`docx/docm`), PowerPoint (`pptx/pptm`), Excel (`xlsx/xlsm/xlsb/xls`) + `ods` (calamine), `odt/odp`, `rtf`, `eml` (mailparse), `pdf`, `html`. Add new types in `extract.rs` (never in a connector); `MIME_KINDS`/`kind_for` maps Drive MIME → kind (octet-stream → filename/named-file fallback). Folder walking (`folder.rs`) is repo-aware: the `ignore` crate honors `.gitignore` even outside a git checkout, keeps dotfiles like `.github/`, and `SKIP_DIRS` prunes `.git`/node_modules/target/build output. Caveat: a user folder sharing one of those names is skipped too (only the watched root is exempt — rename it to ingest it). Drive `plan_for` exports native Google types (Docs→text, Sheets→csv, Slides→text, other→pdf); failures/empty → `skipped` (purge-safe). Legacy `.doc`/`.ppt` are deliberately unsupported.
- **Frontend build type-checks** (`tsc -b && vite build`) — unused vars/imports fail the build (see the
  `ollamaOk` lesson). Keep TS types in `types.ts` in sync with backend schemas or the build breaks.
- **Empty-string `app_settings` rows mean "unset"** — the Settings UI saves `''` for blank fields (e.g. "Local Ollama" leaves `classifier_base_url`/`embed_base_url` empty). Python's `_cached_get` and the Rust port both returned `Some('')` on a cache miss but `None` on a cache hit, so `.or_else(ollama_base_url)` chains worked only within the 3s cache TTL — retrieval flapped `hybrid` ↔ `keyword_only`, `test-connection` failed with "API key required for a cloud embedding provider", and `/system/status` mislabeled local providers as "cloud". Fixed via `settings.rs` `non_empty()`: empty reads as `None` on every path and `set("")` deletes the row. Never reintroduce `Some("")` — `format!("{}/v1/embeddings", "")` is a relative URL and fails.
- **Shell tool gotchas (2026-10-11):** a commit message containing `"` breaks the PowerShell `-m` arguments (git then receives the remainder as a pathspec) — write the message to a file and use `git commit -F <file>`. And `git add <path>` aborts the *entire* add when that path is missing from the worktree (e.g. after `git rm`), so a follow-up commit ends up empty — use `git add -A` (or `git add .`).
- **More shell gotchas (2026-10-11, cost several rounds):** `$ErrorActionPreference = "Stop"` plus a native command that writes to stderr (git, chrome) aborts the whole script — a tag push was silently skipped that way. Capturing chrome's stdout into a *variable* yields nothing (`$x = & chrome --dump-dom …`), while piping to a file works; and chrome forwards arguments to an already-running instance ("Opening in existing browser session"), so dumps silently produce no output. Use `| Out-File` or node's `execFileSync`, and never `Stop-Process chrome` broadly (that closes the owner's browser). Windows Chrome also enforces a ~482px minimum window: a true 390px viewport needs a same-origin iframe. See `docs/handover-2026-10-11-v1.0.19.md`.
- **Agent skills are installed state, not source.** Our own three skills are tracked once, in `.agents/skills/` (the dir every agent reads — see the `skills` CLI table); `.claude/skills/`, `agent/skills/` and the vendored third-party skills are gitignored and restored with `npx skills experimental_install` (`skills-lock.json` is the manifest). Never `git add -A` them back — that is how 896 files / 3.1 MB (81% of the index) got committed once.
- **Repo files are CRLF** — the edit tool matches exact text, so multi-line finds with LF fail; use single-line anchors or a script that preserves CRLF. **Rust builds DO work on this box** (2026-10-12): use `C:\Users\Aleksandr\.local\winlibs\mingw64\bin` on PATH + `cargo +stable-x86_64-pc-windows-gnu ... --target x86_64-pc-windows-gnu --offline` from `rust/` (see `rust/AGENTS.md` + `docs/handover-2026-10-10-b54-verify.md`); the msvc toolchain still cannot link. A fresh `git worktree` has no `rust/target`/`frontend/dist` (gitignored) — robocopy them from the main checkout or rebuild.
- **Backup/export (Phase 25, `rust/crates/anchorcore/src/backup.rs`)** — archives are uid-based (never local ids), records are rendered by SQLite `json_object` and streamed into the zip, import is one transaction (dry run = apply-then-rollback), the secrets bundle is decrypted *before* any write (a wrong passphrase must leave nothing behind), `exports/safety-backup-*` is never swept, and imports queue `jobs.kind='embed'` (`Pipeline::embed_missing_for_source`) — retrieval stays keyword-only until it runs. Keep the plan §7 grep test green.

- **Entity chunks don't surface through `toc_search` leaf search** (`section_id IS NULL` never matches the top-sections filter), so live citations are overwhelmingly *document* chunks — any feature keyed on cited entities (R27.4 conflict warnings) must attribute via the item (`conflicts_for_chunks`), not the hit's `entity_id`. Related: `graph_expand` walks *every* relationship kind at `GRAPH_DEFAULT_WEIGHT` unless told otherwise — a new adversarial kind (e.g. `conflicts_with`) must be explicitly skipped or contradictions enter as supporting evidence.
- **Code splits by symbols, not headings** (R27.1): `chunk_code_with_sections` (python indent + brace keyword/depth families, js-const + c-style extras) returns the same `(SectionMeta, ChunkMeta)` shape via the shared `materialize` tail; `store_doc_chunks` dispatches on `doc_type == "code"` with the filename from `external_id` (opaque ids → root-only). Minified files + unknown extensions stay root-only by guard, never by accident.
- **Configs split by key, not headings** (R33.4): `chunk_config_with_sections` (YAML every-key + JSON/JSONC object keys indent-nested, TOML flat dotted tables + every key a sub-section) keys off `extract::is_config_filename`, *not* doc_type — a `.yaml` splits by keys whatever type-detect says. Configs keep their prose doc_type badge but skip extraction + distill via `ClassifyCtx::skips_extraction` (same skip as `code`); every-key splitting is deliberate (one-line leaf sections pinpoint), lists/comments/markers fold by design.
- **`/qa/search` hits carry provenance and full content on opt-in** (R28.1): `item_title` from `ingested_items.title`, `path` = section breadcrumb else the item's `external_id` (folder: relative file path); `include_content: true` is an alias for `fields: content`, and content is returned verbatim (MCP caps at 4000 chars and flags `content_truncated`). Don't reintroduce the old empty `item_title`/4000-char-capped server content.
- **The embedding backfill is `scheduler.rs` `spawn_backfill`** (R28.2, startup + every 15 min): it embeds NULL-vector chunks **ungated** on purpose — the ingest IDF/PII gates are not re-applied, because this is the safety net that drains `pending_embeddings` (IDF-skipped chunks like `"The"` otherwise stay NULL forever). Failures log + record a `system_events` warning. Don't "simplify" it away, and don't add gating to it; `Pipeline::embed_missing_for_source` (backup import) stays gated.
- **Logging is a stdout + rotating-file pair** (R28.3): `init_logging` writes `{data_dir}/anchorcore.log` with 10 MB × 3 backups (Python `RotatingFileHandler` parity), default level INFO, `RUST_LOG` overrides. Windowed release builds have no console — dropping the file layer makes the app log nothing but panics.
- **Job totals are set by the run phase, not by `finish_job`** (R28.1): sync keeps `total` = docs scanned and `processed` = docs walked; the result JSON's `items` is only the changed/new subset. `finish_job` must never overwrite totals with `items` (that made a 10-file resync with 3 changed files read as 3/3).
- **The update check is manual-first** (R29.1): `GET /system/update` never contacts the network — a plain GET serves the cache (`plain_update_get_never_checks_network`); only `?refresh=1` (60s floor) or the explicit startup opt-in (`update_check_enabled`/`ANCHOR_UPDATE_CHECK_ENABLED`, default off, explicit `0` beats env) performs one GitHub `releases/latest` fetch. UI: System-tab `UpdateCard` + per-version dismissible `UpdateBanner` (both cache-only reads). Don't add a boot check outside the opt-in. `anchorcore --version` (clap `command(version)`) exists for the setup skill's on-disk check; docs: `docs/updates.md`.
- **Demotion must run after RRF fusion, never before** (R32.3): RRF fuses by rank and ignores scores, so demoting the input lists changed scores without reordering hybrid results (1.0.24 retest: fixture scores dropped but the live file still never surfaced). `fuse_and_rank` demotes post-fusion/pre-truncate and `apply_graph` demotes post-graph-fusion; entity chunks carry the effective item (`COALESCE(c.item_id, e.item_id)`) so every hit/citation has an `item_id`. Local test note: run Rust tests with `ANCHOR_SECRETS_NO_KEYRING=1` on a dev Mac or the two no-model-key answer tests read your real keychain key and fail (environmental, CI-clean).
- **Test-env handling is live-first ordering, not just a multiplier** (R32.7/R32.8): single-list BM25 scores span several × on length norm alone, so halving fixture scores never reordered ("scores halved but the order didn't change" — the retest's plain-question and literal-token failures). `prefer_live_envs` stable-partitions live above test directly before every cut (`fuse_and_rank`, `apply_graph`, TOC summary stage); the `×0.5` multiplier only shapes displayed scores. Same guards everywhere: queries naming test files are exempt, `ANCHOR_TEST_ENV_DEMOTE=1.0` disables. `env` (`live`/`test`) classifies per item through `env_for_item` — path markers (incl. Go `testdata/`) or fixture-grade file-head vocabulary (`fixture`/`mock`/`sandbox`/`staging`/`testdata` only; casual "test"/"sample" prose stays live). Named-file boost still wins over env (an explicit filename is the strongest intent signal).
- **Final hits are leaf chunks with match-centered previews** (R32.5/R32.6/R32.9): `section_summary`/`doc_summary` chunks are TOC coarse-stage internals — base vec/FTS, scan fallback, `keyword_fallback` (newest-first would return extracts) and `best_chunk_for_item` all filter to leaf kinds, so a one-line extract can never surface as evidence. Summaries/snippets window on the query terms (`match_snippet`, `…`-marked; head only when nothing matches) instead of head-truncating the expanded bytes. Expansion claims siblings across the page in rank order (seeded with hit ids) — a sibling pulled into an earlier hit's `[continued]` is skipped + flagged by later hits instead of repeating bytes; `expand_by_item` shares the cap/flag.
- **Item discovery is `GET /items`, drill-down is `GET /items/:id`** (R32.11/R32.12): the listing takes `path_prefix` (slash-normalized prefix LIKE, specials escaped) + `source_id` + limit/offset and returns metadata-only `{items, total}` — no bytes, no gate (like the entities list). Item JSON carries both `text` (canonical, backward compat) and `content` (alias — every content-carrying payload exposes `content`). Entities carry `chunk_id` (first own chunk, explicit null when none) next to `item_id` (R32.13); MCP `get_entity` passes `item_id`/`chunk_id`/`item_is_pii` through.
- **The tray needs NSApplication first on macOS** (R34.1): `tray-icon` never initializes `NSApplication`, and bare `NSStatusBar.statusItemWithLength` aborts on macOS 15 (`CGSConnectionByID` assertion, uncatchable, exit 134) — proven by a 20-line probe vs a Swift `NSApplication.shared` probe on the same box. `tray.rs` `init_appkit` mirrors the Swift probe (shared instance only). Related: tray creation must stay on the main thread, so `main` binds via `std::net::TcpListener` (no await) before `spawn_if_enabled` — a second instance also fails the bind before planting a zombie icon. Gating: `--no-tray` / `ANCHOR_TRAY=0`; Linux is a stub (target-gated deps — never add tray system libs to Linux CI). Tray failures log and fall back to tray-less; the server always runs. Docs: `docs/tray.md`.
- **Lizard 1.24 mis-scopes Rust files with non-ASCII char literals** (R36.7): its lifetime regex treats `'ƒ'`-style table entries as lifetime opens, swallowing the rest of the file into one function's span — at HEAD this silently hid `rtf_to_text`'s true CCN 31 (no warning) and reported `cp1252_byte` clean. File growth past the 1000-line span threshold surfaces bogus "new" warnings on untouched functions. Fix is behavior-identical `\u{XXXX}` escapes in lookup tables (see `extract.rs` `cp1252_byte`); if lizard newly flags a function your diff never touched, suspect the tokenizer before the code. `rtf_to_text` CCN 31 is filed as R36.14 (gate hygiene, not retrieval).
- **Purged names stay purged — never write them back, not even in verification specs.** The 2026-10-13 purge replaced client names repo-wide (history rewritten); a NOTICE naming the two removed wordlist entries got mangled by the rewrite (`continental`/`acme` were never entries) and a backlog DoD listing the purged set re-polluted the tree. Describe purged content by count/date only (`comm -23` against the vendored file reveals entries mechanically); grep patterns for re-verification are owner-held, not stored in-repo.
- **Lizard 1.24 also mis-scopes `matches!` char-alternative lists** (2026-10-13): `matches!(c, 'i' | 'v' | …)` swallows the rest of the file into the function's span exactly like the R36.7 artifact (proven by truncation bisection — every fragment parses fine in isolation, the full file doesn't). Fixed behavior-identically (`"ivxlcdm".contains(...)`, same for the `'…'` literals in `toc_entry_core`). Residual: `text_decodes_*` still shows a bogus 592-NLOC span from cumulative state corruption in the test module (span CCN 1 without the PDF tests) — proven artifact, not real complexity; check spans before believing a flag there.
- **PDF extraction is adaptive, not single-pass** (2026-10-13): `pdf-extract`'s plain sink spaces at any glyph gap over 0.1em and ignores `Tc`/`Tw` when predicting the next glyph, so kerned/tracked PDFs shred mid-word ("Cancellati ons") — but only some PDFs trip it. Worse, Word-exported PDFs report **width 0** for positioned glyphs (`i`, `p`, …), so gaps after them measure the full advance (0.24–0.61em) and no threshold can work — the repair sink joins blind gaps (<0.05em widths) unless past 1.0em. `extract_pdf` keeps the standard output unless the split-word detector trips (unknown 2–4-char fragments + rejoinable pairs vs vendored top-30k wordfreq `pdf_words.txt`, trip ≥6 and ≥6‰, calibrated 0/832 clean), then runs the `PdfFlowOutput` repair (0.2em gap, explicit-space breaks, `Tc`-aware advance, blind-gap joins, para breaks, dehyphenation) which wins only on strict improvement. Repairs warn + record `system_events` (`pdf split-word repair`); ties stay debug-level. Force re-sync re-extracts already-indexed PDFs. Word list is Apache-2.0 (`pdf_words.NOTICE`).
- **B12 cleaning is prose-only; `chunk_text` snaps to boundaries** (2026-10-13): `clean_doc_text` (page numbers, controls, repeated headers — archived `cleaning.py` parity incl. banker's rounding) runs in `store_doc_chunks` only when `!skips_extraction()` — collapsing whitespace destroys YAML nesting and Python indentation, so code/config keep raw text (pinned by the config breadcrumb test). `chunk_text` pulls window ends back to sentence ends else word breaks (bounded by overlap, tail always covered) and advances starts to word starts clamped to the previous end — same coverage, no mid-word/mid-sentence cuts. `classify_windows` keeps the old blind loop (LLM windows don't need it).

## UI notes

- `frontend/src/App.tsx` — tabs + header (project picker scopes Ask via B15); mounts `OnboardingWizard`
  when `/system/onboarding` reports `needs_wizard` (first launch, no sources) and it isn't dismissed
  (`localStorage['wizard-dismissed']`).
- `frontend/src/OnboardingWizard.tsx` — B8: checks (Ollama install/pull per platform) → connect a folder
  or the bundled `sample/` corpus (sync with live progress) → ask a question with cited answer.
  Sample path resolves via `Path(__file__).resolve().parents[3] / "sample"` (dev checkout only; frozen
  apps report `sample.available: false`).
- `frontend/src/FolderPicker.tsx` — folder browser for folder sources ("Browse…" in the SourceForm add/edit branch + wizard step 2). Talks to `GET /system/fs` (`fsbrowse.rs`: child dirs only, roots view = Home + drives, 400 `detail` on relative/missing; never canonicalize → no `\\?\` leak). Keep the typed-path input — pasting a known path stays fastest.
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
- Website (`website/`): CI auto-deploys to anchorcore.dev on `v*` tags when `website/` changed since the previous tag (`release.yml deploy-website`, needs `CLOUDFLARE_API_TOKEN` + `CLOUDFLARE_ACCOUNT_ID` secrets); manual fallback is `cd website && npm run build && npx wrangler deploy`. Landing-page examples must come from the repo's `sample/` (payments, the default) or `sample-ai/` (AI startup, `?corpus=ai`) corpora — never `business-scale` leftovers (`ACME`, `§434`) in user-facing copy. `/.webmcp/bridge.js` is injected by the Cloudflare edge (WebMCP), not a repo asset — disclosed in `/privacy` and documented in `website/README.md` (R18.10).
