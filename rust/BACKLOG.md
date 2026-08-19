# Rust Port — Backlog (B31)

> Source of truth for incremental Rust port. Each task is sized for a single AI agent (parallel, no file overlap). See `docs/rust-port.md:104` plan and `rust/README.md:1`.

Status: `todo` | `doing` | `done`. Update this file when you pick/complete a task. Branch per task: `rust/TASK_ID`.

---

## Phase 0 — Gate (0 days, do cheaper wins first)

| ID | Title | DoD | Files |
|---|---|---|---|
| R0.1 | Perf spike: `vec0` in Python | Retrieval uses `vec_chunks` vec0 (already `b33a0c1`), Python-scan fallback, `/system/status.retrieval` reports `vec0` vs `scan` + p50. Bench on real corpus; record latency before Rust. | `backend/app/answer_engine.py`, `backend/tests/test_retrieval.py` |
| R0.2 | Bundled inference decision | Evaluate llama.cpp sidecar vs B8 wizard. Doc decision in `rust/docs/decisions.md`. | `docs/packaging.md`, `rust/docs/decisions.md` |

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
| R2.2 | Answer orchestration (ask) | 1d | R2.1 R1.5 | `POST /qa` + `POST /qa/public` (B30 gate), project scoping `source_ids`, follow-up rewrite skip when untrusted, citations. | todo |
| R2.3 | Conformance harness | 0.5d | R1.3 R2.2 | `backend/tests` runs against `cargo run -p anchorcore -- --port 8123` (black-box HTTP), `test_retrieval.py` green. | todo |

## Phase 3 — Connectors & pipeline (1–2 weeks)

| ID | Title | Est | Dependencies | DoD |
|---|---|---|---|---|
| R3.1 | Folder connector + watcher | 1d | R1.2 | `notify` crate, `path` config, same `IngestionDoc` shape, hash dedup |
| R3.2 | Jira connector (410-safe) | 0.5d | R1.2 | `GET /search/jql` with `nextPageToken`/`isLast` fallback `/search` `startAt/total`, `project in (...)` for multi, `BaseConnector::list_projects` -> `POST /sources/jira/projects` checkbox |
| R3.3 | GDrive connector | 0.5d | R1.2 | `folder_id` + Bearer token, same mock pattern as `tests/test_gdrive.py` |
| R3.4 | Classification orchestration | 1d | R1.5 | Calls Ollama/cloud via `reqwest`, `cloud_trusted` gate (B39), rule fallback, `window_hash` skip |
| R3.5 | Chunking / hashing / distill | 1d | — | Port `chunking.py`/`hashing.py`/`distill.py` + `_flag_pii` + `is_pii`/`_dismissed` sentinel, `pii_categories` |
| R3.6 | Embedder + IDF gate | 1d | R3.5 | `signal()` gate, `pack_f32`, cloud-trust gate for `is_pii` chunks, `vec0` sync triggers |

## Phase 4 — Jobs & system (1 week)

| ID | Title | Est | Dependencies | DoD |
|---|---|---|---|---|
| R4.1 | JobManager (bounded) | 0.5d | R1.2 | `MAX_CONCURRENT=2`, `pending` queue, `running`/`cancel`, `POST /sources/{id}/sync` 202 + poll |
| R4.2 | Scheduler (poll loops) | 0.5d | R4.1 | `jira_poll_minutes` / `folder_scan_minutes`, `reload_sources` on CRUD |
| R4.3 | PII config + review | 0.5d | R3.5 | `GET/PUT /pii/config`, `GET /pii/review` (`only_flagged` + `_dismissed` hide), `POST /pii/review/{id}` (Mark PII / Not PII), `POST /pii/scan/{id}` |
| R4.4 | Entities / review / projects | 1d | R1.3 | `GET /entities/{id}/context` B27, low-confidence/duplicates/merge, `projects` scoped search B15 |
| R4.5 | MCP sidecar (B14.1) | 1d | R2.2 | `anchorcore_mcp.py` equivalent `mcp/server.py` 6 read-only tools via stdio |

## Phase 5 — Packaging & cutover

| ID | Title | Est | Dependencies | DoD |
|---|---|---|---|---|
| R5.1 | Embed frontend/dist | 0.5d | R1.3 | `include_dir!` embeds `frontend/dist`, serves last, `cargo build --release` yields single binary |
| R5.2 | Cross-compile + release | 0.5d | R5.1 | `cargo cross` win/mac/linux, ad-hoc `codesign`, `dist/AnchorCore-*` artifacts, `release.yml` |
| R5.3 | Cutover checklist | 0.5d | R4.* | All `backend/tests` green against Rust, Python retires, version bumps in `rust/Cargo.toml` (single source) |

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
