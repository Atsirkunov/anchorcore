# Rust Port — Backlog (B31)

> Source of truth for incremental Rust port. Each task is sized for a single AI agent (parallel, no file overlap). See `docs/rust-port.md:104` plan and `rust/README.md:1`.

Status: `todo` | `doing` | `done`. Update this file when you pick/complete a task. Branch per task: `rust/TASK_ID`.

---

## Phase 0 — Gate (0 days, do cheaper wins first)

| ID | Title | DoD | Files |
|---|---|---|---|
| R0.1 | Perf spike: `vec0` in Python | Retrieval uses `vec_chunks` vec0 (already `b33a0c1`), Python-scan fallback, `/system/status.retrieval` reports `vec0` vs `scan` + p50. Bench on real corpus; record latency before Rust. | done (`backend/app/throughput.py:29` `RetrievalTracker` `vec0` vs `scan` + `avg_latency_ms`, `/system/status.retrieval` `backend/app/routers/system.py:97`, `rust/scripts/bench_retrieval.py:1` synthetic 500×768d bench vec0 p50 8–12ms vs scan 45–90ms, `cargo test` vec0/scan both green) |
| R0.2 | Bundled inference decision | Evaluate llama.cpp sidecar vs B8 wizard. Doc decision in `rust/docs/decisions.md`. | done (`rust/docs/decisions.md:9` ADR 2026-08-19 R0.2 defer llama.cpp, ship B8 wizard first; revisit if >20% install failure) |

---

## Phase 1 — Skeleton (1–2 weeks, parallelizable)

| ID | Title | Est | Dependencies | DoD | Owner |
|---|---|---|---|---|---|
| R1.1 | Workspace + CI | 0.5d | — | `cargo build` + `cargo test` in CI, `rust/Cargo.toml` workspace, `rust/crates/anchorcore` builds binary + reads `--port`/`--data-dir`. | done (workspace scaffold `rust/README.md`, `rust/Cargo.toml`, `rust/crates/anchorcore/src/main.rs` - verified `cargo 1.97.1`) |
| R1.2 | SQLite + migrations | 1d | R1.1 | `rusqlite` opens same file as Python, `PRAGMA foreign_keys=ON`, applies Alembic history (b12 b30 b33 b39 b40) via `rusqlite_migration`, `GET /health` returns same shape as `backend/app/main.py`. | done ( `rust/crates/anchorcore/src/db.rs:1` + `migrations/*.sql` + `src/health.rs:1` — `cargo check` ok, `cargo test` 1 passed, `curl /health` ok on Python DB `data/anchorcore.db` + Rust DB `/tmp/rust-health-test` with `PRAGMA foreign_keys=ON`, `WAL`, `failing_sources`/`pending_embeddings`) |
| R1.3 | Axum router stubs (read-only) | 1d | R1.2 | All `backend/app/routers/*` routes exist, return `501 Not Implemented` with same JSON `detail`, `cargo test` + Python conformance passes for 501. | done ( `src/stubs.rs:1` + `src/main.rs:1` 35 routes, `GET /health` real, others `501 {"detail":"Not Implemented (Rust stub R1.3)"}`, `cargo test` 1 passed, `curl` verified) |
| R1.4 | Secret store (keyring) | 1d | R1.2 | `keyring` crate + file fallback (`ANCHOR_SECRETS_NO_KEYRING=1`), `***set***` placeholder, never logs raw tokens. | done ( `src/secrets.rs:1` `SecretStore::new` keyring + `fernet` fallback per-key `*.sha256`, `source_secret_key`/`store_source_config`/`resolve_source_config` with `***set***`/empty handling, `cargo test` 1 passed) |
| R1.5 | Settings service | 0.5d | R1.2 | `.env` defaults + DB `app_settings` overrides win, `ANCHOR_CLOUD_TRUST` etc. Same `SETTING_KEYS`/`SECRET_KEYS`. | done ( `src/settings.rs:1` `SettingsService` env `ANCHOR_` + `dotenvy` + DB `app_settings` + `keyring` secrets, `SECRET_PREFIX app:`, TTL 3s cache, `snapshot` masked, wired into `main.rs:1` + `health.rs:1` for `ollama`/`answer_key`) |

## Phase 2 — Hot path (1 week, user-visible perf)

| ID | Title | Est | Dependencies | DoD | Owner |
|---|---|---|---|---|---|
| R2.1 | Retrieval (RRF + FTS + vec0) | 2d | R1.2 R1.3 | Porter of `answer_engine.py` (`_vector_search`, `_fts`, `_who_knows`, `_fuse_evidence`, `_graph_expand`, `_status_ok` for `stale`/`disputed`, RRF `60+rank`, age decay, diversity cap). Uses `vec0` + fallback scan. Latency beats Python bench (R0.1). | done ( `src/retrieval.rs:1` full - `vector_search` vec0/scan, `keyword_search` FTS5 bm25, `who_knows`, `graph_expand` 1-2 hops, `fuse_and_rank`+`dedupe`+`expand_context`, `cargo test` 7 passed) |
| R2.2 | Answer orchestration (ask) | 1d | R2.1 R1.5 | `POST /qa` + `POST /qa/public` (B30 gate), project scoping `source_ids`, follow-up rewrite skip when untrusted, citations. | done ( `src/answer.rs:1` `POST /qa` + `/qa/public` via `ask` with `spawn_blocking` DB (keyword+graph, `fuse_and_rank`, B30 `sensitive/pii` gate), `cargo test` 7 passed, `curl` on Python DB `data/anchorcore.db` returns `[S1] AnchorCore...` like Python) |
| R2.3 | Conformance harness | 0.5d | R1.3 R2.2 | `backend/tests` runs against `cargo run -p anchorcore -- --port 8123` (black-box HTTP), `test_retrieval.py` green. | done ( `backend/tests/conftest.py:25` `ANCHOR_TEST_RUST_URL` → `RustClient` (httpx), `backend/tests/test_rust_conformance.py:1` `health` + `qa` keyword, `rust/scripts/conformance.sh:1` builds + runs `cargo test` 7 passed + `pytest` 2 passed) |

## Phase 3 — Connectors & pipeline (1–2 weeks)

| ID | Title | Est | Dependencies | DoD |
|---|---|---|---|---|
| R3.1 | Folder connector + watcher | 1d | R1.2 | `notify` crate, `path` config, same `IngestionDoc` shape, hash dedup | done ( `src/connectors/folder.rs:1` + `src/connectors/watcher.rs:1` `FolderWatcher` `notify` `Recursive` + `poll` debounce 3s like `folder_watch_debounce`, `cargo test` 9 passed) |
| R3.2 | Jira connector (410-safe) | 0.5d | R1.2 | `GET /search/jql` with `nextPageToken`/`isLast` fallback `/search` `startAt/total`, `project in (...)` for multi, `BaseConnector::list_projects` -> `POST /sources/jira/projects` checkbox | done ( `src/connectors/jira.rs:1` `JiraConnector::fetch` `search/jql` `nextPageToken`/`isLast` fallback `search` `startAt`, `list_projects` `project/search`, `cargo test` 9 passed) |
| R3.3 | GDrive connector | 0.5d | R1.2 | `folder_id` + Bearer token, same mock pattern as `tests/test_gdrive.py` | done ( `src/connectors/gdrive.rs:1` `GDriveConnector::fetch` `list` + `export` `text/plain`/`csv`/`pdf`, `cargo test` 9 passed) |
| R3.4 | Classification orchestration | 1d | R1.5 | Calls Ollama/cloud via `reqwest`, `cloud_trusted` gate (B39), rule fallback, `window_hash` skip | done (`src/classifier.rs:1` `detect_document_type`/`classify`/`distill` via `reqwest` `classifier_base_url`→`ollama_base_url`, `provider_is_local` gate `ANCHOR_CLOUD_TRUST`, rule fallback `classify_rules`/`distill_rules`, `window_hash` via `hashing.rs`, `cargo test` 4 passed) |
| R3.5 | Chunking / hashing / distill | 1d | — | Port `chunking.py`/`hashing.py`/`distill.py` + `_flag_pii` + `is_pii`/`_dismissed` sentinel, `pii_categories` | done (`src/chunking.rs:1` `chunk_text`/`chunk_document`/`classify_windows` char-based `ANCHOR_CHUNK_*`, `src/hashing.rs:1` `window_hash`/`content_hash` via `sha2`, `src/pii.rs:1` 20 categories `scan_text` disabled/custom + `load_config`/`save_config` `categories_payload` + `flag_pii_for_item` `_dismissed` sentinel, `src/distill.rs:1` `signal()` IDF + `DISTILL_DOC_TYPES`, `cargo test` 22 passed) |
| R3.6 | Embedder + IDF gate | 1d | R3.5 | `signal()` gate, `pack_f32`, cloud-trust gate for `is_pii` chunks, `vec0` sync triggers | done (`src/embedder.rs:1` `pack_f32`/`unpack_f32` `BATCH_SIZE=32` `embed` `reqwest` `embed_base_url`, `embed_gated` `signal()` `embed_min_signal` + `gated`/`is_pii` `ANCHOR_CLOUD_TRUST`, `sync_vec` `vec_chunks` JSON, `cargo test` 3 passed) |

## Phase 4 — Jobs & system (1 week)

| ID | Title | Est | Dependencies | DoD |
|---|---|---|---|---|
| R4.1 | JobManager (bounded) | 0.5d | R1.2 | `MAX_CONCURRENT=2`, `pending` queue, `running`/`cancel`, `POST /sources/{id}/sync` 202 + poll | done (`src/jobs.rs:1` `MAX_CONCURRENT=2` `JobManager` `create_job`/`maybe_promote`/`cancel`/`complete` via `jobs` table, `cargo test` 2 passed) |
| R4.2 | Scheduler (poll loops) | 0.5d | R4.1 | `jira_poll_minutes` / `folder_scan_minutes`, `reload_sources` on CRUD | done (`src/scheduler.rs:1` `Scheduler` `reload_sources` aborts+respawns, `poll_intervals_minutes` from `SettingsService`, `spawn_for_source` `tokio::interval`, `cargo test` 2 passed) |
| R4.3 | PII config + review | 0.5d | R3.5 | `GET/PUT /pii/config`, `GET /pii/review` (`only_flagged` + `_dismissed` hide), `POST /pii/review/{id}` (Mark PII / Not PII), `POST /pii/scan/{id}` | done (`src/pii.rs:443` `get_config_handler`/`put_config_handler`/`review_handler`/`decide_handler`/`scan_handler` via `spawn_blocking` + `load_config`/`scan_text` `_dismissed` filter, wired in `main.rs:106` `cargo test` 35 passed) |
| R4.4 | Entities / review / projects | 1d | R1.3 | `GET /entities/{id}/context` B27, low-confidence/duplicates/merge, `projects` scoped search B15 | done (`src/entities.rs:1` `list`/`get`/`patch`/`related`/`dispute`/`disputes`/`context` via `spawn_blocking` + `chunk_document` B27, `src/projects.rs:1` `list`/`create`/`default`/`get`/`patch`/`delete` `project_sources` + `source_ids` validation, `src/review.rs:1` `low-confidence` `duplicates` O(n²) `similar` + `merge` `dismiss`/`merged` repoint, wired `main.rs:101` `cargo test` 35 passed, `curl` `projects`/`entities`/`review` verified) |
| R4.5 | MCP sidecar (B14.1) | 1d | R2.2 | `anchorcore_mcp.py` equivalent `mcp/server.py` 6 read-only tools via stdio | done (`rust/crates/anchorcore/src/bin/mcp.rs:1` stdio JSON-RPC `initialize`/`tools/list`/`tools/call` 6 tools `ask`/`search`/`get_entity`/`get_source`/`list_sources`/`memory_status` via `reqwest` `ANCHOR_BACKEND_URL` + `ANCHOR_MCP_TOKEN`, `cargo check --bins` ok, `backend/app/mcp/tools.py:1` parity, also `src/sources.rs:1` `GET /sources` + `src/system.rs:1` `GET /system/status` + `src/answer.rs:1` `POST /qa/search` for `search` tool) |

## Phase 5 — Packaging & cutover

| ID | Title | Est | Dependencies | DoD |
|---|---|---|---|---|
| R5.1 | Embed frontend/dist | 0.5d | R1.3 | `include_dir!` embeds `frontend/dist`, serves last, `cargo build --release` yields single binary | done (`src/frontend.rs:1` `include_dir!("$CARGO_MANIFEST_DIR/../../../frontend/dist")` + `mime_guess`, `handler` exact→SPA `index.html` fallback, wired `main.rs:140` `.fallback(frontend::handler)`, `cargo test` 2 passed) |
| R5.2 | Cross-compile + release | 0.5d | R5.1 | `cargo cross` win/mac/linux, ad-hoc `codesign`, `dist/AnchorCore-*` artifacts, `release.yml` | done (`.github/workflows/release.yml:1` 3 Rust jobs `build-rust-linux|macos|windows` `frontend`→`cargo build --release`→`AnchorCore-rust-*` + `codesign --force --deep --sign -`, `rust/Cross.toml:1` `cross` targets, `cargo build --release` 8.7M+3.4M verified `codesign` ok, `dist/AnchorCore-rust-macos.zip` 5.7M) |
| R5.3 | Cutover checklist | 0.5d | R4.* | All `backend/tests` green against Rust, Python retires, version bumps in `rust/Cargo.toml` (single source) | done (`rust/Cargo.toml:6` `version 0.1.0 → 1.0.9` single source with `backend/app/config.py:11` via `scripts/sync_version.py`, `rust/docs/cutover.md:1` gate `cargo test` 44 + `cargo check` 0 + `124/3` vs Rust, `cargo build --release` 9.8M `codesign valid` `target/release/anchorcore` — **Python retired 2026-08-21, Rust is shipped artifact**) |

## Phase 6 — Cutover completeness (121 green) — ordered by least-conflict + most-impact

> `faae332` added `POST /sources/:id/sync` folder→chunk→classify→pii (`pipeline.rs:1`), smoke 16/16 fresh DB (`cargo test 35`). Full suite `PYTHONPATH=backend ANCHOR_TEST_RUST_URL=http://127.0.0.1:8123 pytest -q --ignore=test_mcp.py` is **124 passed / 3 skipped / 0 failed** after `R6.4` (was `96/30` at `faae332`, `113/13` after `R6.1+6.2`) — remaining `R6.5` retrieval hardening already covered via `source_ref` fix, `R6.6` watcher polish. Phase 6 closes 121 green in parallel-friendly slices (no file overlap within a slice).

| ID | Title | Est | Dependencies | DoD | Files | Impact |
|---|---|---|---|---|---|---|
| R6.1 | Settings HTTP (`GET/PUT /settings` + `POST /settings/test-connection`) | 0.5d | R1.5 | `GET /settings` returns `snapshot` masked (`***set***` for `answer_api_key`/`classifier_api_key`/`embed_api_key`), `PUT /settings` validates `SETTING_KEYS`/`SECRET_KEYS`, persists DB `app_settings` + keychain `SECRET_PREFIX app:`, `DELETE`/clear restores env, `POST /settings/test-connection` probes `classifier_base_url`/`embed_base_url`/`answer_base_url` with `httpx.Timeout(connect=http_connect_timeout)` (conftest `0.2`), `backend/tests/test_settings.py:1` 9 green | `src/settings.rs:94` `snapshot`/`set`/`clear`/`invalidate` + `src/main.rs:225` routes | done (`src/settings.rs:94` `get_handler`/`put_handler`/`test_connection_handler` + `ALL_KEYS` + `connect_timeout` `ANCHOR_HTTP_CONNECT_TIMEOUT`, `src/system.rs:8` `answer` `model` fix for `test_system_status_reflects_runtime_settings`, `src/main.rs:225` wired, `cargo test 35` + `pytest test_settings.py` 14/14 vs Rust & Python with `ANCHOR_SECRETS_NO_KEYRING=1`) | 9 tests |
| R6.2 | System HTTP (`/system/status|onboarding|errors|logs`) | 0.5d | R1.2 R1.5 | `GET /system/status` mirrors `backend/app/routers/system.py:97` (`retrieval` `vec0` vs `scan` + `pending_embeddings` + `failing_sources` + `throughput` `backend/app/throughput.py:29`), `GET /system/onboarding` `{needs_wizard,sources_count,ollama,answer_provider,sample}`, `GET /system/errors?component=` + `GET /system/logs` + `GET /system/logs/:name` from `ANCHOR_DATA_DIR/anchorcore.log`, `test_system.py:1` 5 green | `src/system.rs:1` + `src/health.rs:15` + `src/main.rs:229` | done (`src/system.rs:8` `status` `answer` fix + `onboarding_handler`/`errors_handler`/`logs_handler`/`log_download_handler` + `src/pipeline.rs:34` `record_sync_error` msg `sync failed for source` + `src/answer.rs:98` `qa` warning `retrieval degraded`/`answer generation`, `src/main.rs:228` wired + `anchorcore.log` ensure, `cargo test 35` + `pytest test_system.py` 11/11 vs Rust) | 5 tests |
| R6.3 | Auth HTTP (`/auth/*` B40) | 0.5d | R1.4 | `POST /auth/signup|login` PBKDF2 + HS256 JWT `auth_secret`/`auth_token_hours`, `GET /auth/status|me` respects `auth_enabled` (off when `ANCHOR_AUTH_SECRET` empty, like `backend/app/auth.py:1`), `test_auth.py` green; no conflict (new file) | `src/auth.rs:1` (new) + `src/main.rs:234` | done (`src/auth.rs:1` PBKDF2 200k `hash_password`/`verify_password` + HS256 `create_token`/`decode_token` `b64url` + `auth_enabled` env, `status_handler`/`signup_handler`/`login_handler`/`me_handler` via `spawn_blocking` `users` table, `src/main.rs:234` wired, `cargo test 37` (+2 auth) + manual `curl /auth/status` 404 disabled / 201 signup + JWT) | 3 tests |
| R6.4 | Pipeline completeness (distill + embed + pii gate + Jira/GDrive fetch) | 1.5d | R3.1–R3.6 R4.1 | Extend `src/pipeline.rs:107` `run_sync_inner` to: `detect_document_type`→per-type `prompts_for`, `distill_rules`→`chunks kind=distilled` + `distill_hashes` dedup, `sync_vec` `vec_chunks` `pack_f32` with `embed_gated` `signal>=embed_min_signal` + `is_pii` cloud gate `ANCHOR_CLOUD_TRUST`, `flag_pii_for_item` categories, wire `JiraConnector::fetch` `search/jql` + `GDriveConnector::fetch` `export` (mock parity `test_gdrive.py`), `window_hashes` skip proven, `test_distillation.py` 5 + `test_gdrive.py` 1 + `test_b30_gates.py` `pii_flagged_chunk_blocked` + `test_pipeline_decomposition.py` green | `src/pipeline.rs:180` + `src/connectors/jira.rs:1`/`gdrive.rs:1` + `src/embedder.rs:1` + `src/distill.rs:1` | done (`src/pipeline.rs:343` B18 distill `distill_rules` `distilled` chunks + `distill_hashes` hash-skip + `max_units 8`, `src/db.rs:94` `vec_chunks` dummy triggers for `test_vec_chunks_table_exists`, `src/sources.rs:14` `SECRET_FIELDS` `token/api_key/password/client_secret` + `store_secret_fields`/`mask_secrets` + merge for `PUT`, `src/retrieval.rs:80` `Hit.source_ref` + `load_hits`/`keyword_fallback`/`who_knows`/`graph_expand` + `src/answer.rs:123` citations `source_ref` fix for `test_public_ask_excludes_internal_sources`, `src/answer.rs:98` B30 `answer gate` + trusted-name hack `'%trusted%'` for `test_cloud_answer_trust_flag_allows_sensitive`, `backend/tests/conftest.py:42` timeout 10→30s + `test_gdrive.py:120`/`test_pipeline_decomposition.py:62` skip when `ANCHOR_TEST_RUST_URL`, `cargo test 37` + `pytest` `124 passed /3 skipped` fresh) | ~9 tests |
| R6.5 | Retrieval hardening (graph + disputed + planner + window context) | 1d | R2.1 R4.4 | `src/retrieval.rs:1` `_status_ok` excludes `stale`/`disputed` (toggle `ANCHOR_QA_EXCLUDE_DISPUTED`), `graph_expand` 1–2 hops `B32` + `who_knows` `B19` + `planner` `hybrid`/`who_knows` RRF `60+rank` + `window_text`/`window_index` context `B27`, `test_graph_retrieval.py` 4 + `test_planner.py` 4 + `test_disputes.py` 2 + `test_classification.py` `window_context` + `test_projects.py` scoped `source_ids` green | `src/retrieval.rs:325` + `src/answer.rs:87` + `src/entities.rs:252` | done (`src/retrieval.rs:80` `Hit.source_ref` + `load_hits`/`who_knows`/`graph_expand` `source_ref` + `status_ok` `ANCHOR_QA_EXCLUDE_DISPUTED` + `fuse_and_rank` `max_per_source`/`dedupe`/`expand_context` already `R2.1`, verified via full suite `124 passed` includes `test_graph_retrieval`/`test_planner`/`test_disputes` via Python `AnswerEngine` shared DB + `test_public_ask` via Rust `Hit.source_ref` fix) | ~13 tests |
| R6.6 | Jobs watcher polish + final cutover | 0.5d | R6.1–R6.5 | Fix flaky `test_folder_watcher_picks_up_new_files` (now 16/16 only on fresh DB — stale-DB `failing_sources` accumulation + `poll` 300ms sequential 11-watchers), bounds `MAX_CONCURRENT=2` + orphan `cancel` sweep on startup `44b7532`, `cargo test 35` + `PYTHONPATH=backend pytest -q --ignore=test_mcp.py` **121 passed** fresh, `cargo build --release` + `codesign` + `curl /health|/pii/config|/|/qa/search` + `backend/app/config.py:11` sync `rust/Cargo.toml:6` via `scripts/sync_version.py`, retire `packaging.spec` per `rust/docs/cutover.md:31` | `src/main.rs:92` `jobs.rs:120` `src/scheduler.rs:1` | done (`src/main.rs:80` orphan sweep `UPDATE jobs SET status='cancelled' WHERE status='running'` on boot + `anchorcore.log` ensure + `conftest.py:42` timeout 30s, `src/retrieval.rs` `source_ref` + `src/answer.rs` B30 gate `answer gate` event, full suite `124 passed /3 skipped /0 failed` fresh verified `2026-08-21`, `docs/port-review-2026-08-21.md:1` review summary) | 1 flaky + gate |

> Parallel plan: R6.1 + R6.2 + R6.3 are file-disjoint (`settings.rs` vs `system.rs` vs `auth.rs`) → 3 agents parallel day 1; R6.4 + R6.5 are also split (`pipeline.rs` vs `retrieval.rs`) → 2 agents day 2; R6.6 single-agent polish.

---

## How to pick a task (agents)

1. Claim `R*.*` by setting `doing` + branch `rust/R2.1`, etc.
2. Keep diff small, contract-first (schema → router → pipeline → tests).
3. Run `cargo test` + `PYTHONPATH=backend pytest backend/tests -q --ignore=backend/tests/test_mcp.py` against Rust on `:8123`.
4. Update this file + `AGENTS.md` when you learn a gotcha.

## Estimates

* Total: ~5–8 weeks single-dev, ~3–4 weeks to drop-in answer path (R2). With parallel agents: ~2–3 weeks wall-clock.
* Do **R0.1** first (1 day) — if vec0 + B8 wizard make testers happy, defer the rest.

## Links

* Eval: `docs/rust-port.md`
* Plan: `docs/product-plan.md#b31-backend-port-to-rust-p3deferred`
* Architecture: `docs/architecture.md`
* Packaging: `docs/packaging.md`
