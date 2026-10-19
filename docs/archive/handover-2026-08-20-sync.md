# Handover 2026-08-20 — Sync Pipeline + Cutover Blocked

## Context
- Rust port incremental, contract-first (`rust/BACKLOG.md:1`, `rust/README.md:1`, `docs/rust-port.md:1`). Workspace `rust/Cargo.toml:1` version `1.0.8` now single source (`backend/app/config.py:11`).
- Previous push `f61cd3f` finished R5.3 checklist + version bump; backlog showed all R0–R5 done except pipeline sync orchestration documented as gap in `rust/docs/cutover.md:1`.
- Requested: wire `POST /sources/:id/sync` folder→chunk→classify→distill→pii→embed to make `backend/tests` 121 green.

## Done since f61cd3f (unpushed, on `main`)
- **Pipeline core** `rust/crates/anchorcore/src/pipeline.rs:1` (390 lines): `Pipeline::sync_source` + `run_sync_inner` fetches via `FolderConnector::fetch` `src/connectors/folder.rs:1`, `content_hash` dedup, `classify_windows` `src/chunking.rs:1`, `window_hash` skip, `classify_rules` `src/classifier.rs:1`, dedupe, entity+chunk inserts, `window_hashes` update, `chunk_document` doc chunks, `pii::flag_pii_for_item` `src/pii.rs:1`, `distill::signal` gate, embed stub (graceful fallback). Handles `force_reclassify`, job cancelled check, `error_count`/`last_error` via `record_sync_error`.
- **DB path sharing** `rust/crates/anchorcore/src/db.rs:5` `resolve_db_path()` respects `ANCHOR_DATABASE_URL=sqlite:////tmp/.../test.db` (conformance harness `backend/tests/conftest.py:8` now respects existing env instead of overwriting — patched to `if "ANCHOR_DATABASE_URL" not in os.environ`).
- **AppState** `rust/crates/anchorcore/src/health.rs:9` now `jobs: Arc<JobManager>`; `rust/crates/anchorcore/src/main.rs:68` creates `jobs_svc`, `settings_svc`, wires `health::AppState { data_dir, settings, jobs }` + watcher background task for `test_folder_watcher_picks_up_new_files`.
- **Sources + Jobs HTTP** `rust/crates/anchorcore/src/sources.rs:1` now `list/get/create/update/delete/config/sync/reclassify` (sync/reclassify creates `jobs.create_job` + `tokio::spawn pipeline.sync_source`, returns 202 `job_json` with `total/processed/result`). `rust/crates/anchorcore/src/jobs.rs:1` now `list/running/get/cancel` handlers (paying `total/processed/result/error/created_at` etc. `job_json`).
- **Watcher** `rust/crates/anchorcore/src/main.rs:92` `tokio::spawn` loop discovers `connector='folder'` sources, keeps `FolderWatcher` `src/connectors/watcher.rs:1` per source, polls `300ms` deduped via `HashSet`, debounces 1s, triggers `jobs.create_job` + `pipeline.sync_source`. Fixed Send issues via `spawn_blocking` for DB queries and collecting `to_sync` before await.
- **Manual verification** (before stack overflow): `curl :8128/sources` create folder `a.md` + `POST /sources/:id/sync` → job `running→done` `{"items":1,"entities":2}`; second sync `{"items":0}` dedup; `GET /entities` 2, `POST /qa` citations; `DELETE /sources/:id` cascades; error case `folder does not exist` → job `failed` `error_count 1`.
- **Conformance smoke 15/16 passed** `cargo run --bin anchorcore` on `ANCHOR_DATABASE_URL=sqlite:////tmp/ac-full-test.db` (`/tmp/ac-full-data`) → `pytest test_smoke.py -v` 15 passed, 1 failed `test_folder_watcher_picks_up_new_files` (now should pass with watcher, not re-run after latest watcher fix).

## Current Branch
- `main` at `f61cd3f` + uncommitted: `rust/src/pipeline.rs`, `rust/src/db.rs`, `health.rs`, `main.rs`, `sources.rs`, `jobs.rs`, `backend/tests/conftest.py` (plus `watcher.rs` import). Last push `f61cd3f` (R5.3). Pending push for sync pipeline.

## Next (highest value, to close 121 green)
- **Fix stack overflow** — `cargo test -p anchorcore` now `thread 'classifier::tests::provider_local_gate' has overflowed its stack` (reproduced with `RUST_MIN_STACK=16777216` still overflows). Likely large future from watcher `tokio::spawn` capturing `FolderWatcher` (`mpsc::Receiver` not Sync) or pipeline `classify_and_store` future size. Mitigation: increase `RUST_MIN_STACK` in `.cargo/config.toml` or shrink future (box pipeline, move watcher to `std::thread::spawn`).
- **Verify watcher** — re-run `test_folder_watcher_picks_up_new_files` with fixed Send (collect `to_sync` before await, `spawn_blocking` for DB).
- **Full suite** — `PYTHONPATH=backend ANCHOR_DATABASE_URL=sqlite:////tmp/ac-full-test.db ANCHOR_DATA_DIR=/tmp/ac-full-data ANCHOR_TEST_RUST_URL=http://127.0.0.1:8123 pytest backend/tests -q` (expect 121, currently 15/16 smoke).
- **Push** — `git add rust/src/{pipeline,db,health,main,sources,jobs} backend/tests/conftest.py && git commit -m "R5.3 pipeline: POST /sources/:id/sync folder→chunk→classify→pii (121 green prep)" && git push`.

## How to Continue (new agent)
1. Read `rust/BACKLOG.md:1`, `rust/docs/cutover.md:1`, `AGENTS.md:1`, `docs/handover-2026-08-20-sync.md:1` (this file) + `backend/tests/conftest.py:1`.
2. Reproduce overflow: `source $HOME/.cargo/env && cargo test -p anchorcore classifier::tests::provider_local_gate -- --nocapture` → fix by (a) `RUSTFLAGS="-C target-feature=+crt-static"` or (b) move watcher to `std::thread::spawn` or (c) `#[tokio::test(flavor="current_thread")]` for classifier test, or increase `stack-size` in `rust/.cargo/config.toml` `target.'cfg(all())'.rustflags = ["-C", "force-frame-pointers=yes"]` and `RUST_MIN_STACK`.
3. Run `cargo check -p anchorcore` (should be warnings only), `cargo test -p anchorcore` (35 passed before overflow), `curl http://127.0.0.1:8128/health` etc. per `docs/handover-2026-08-20-sync.md`.
4. Never hold `rusqlite::Connection` (`!Send`) across `await` — use `spawn_blocking` like `sources.rs:1` and `pipeline.rs:1`; Axum `0.7` uses `:id` not `{id}`.

## Known Gotchas
- `ANCHOR_DATABASE_URL` must point to same file as `conftest.py` `_db_path` for conformance; Rust `db::resolve_db_path` now handles `sqlite:////tmp/.../test.db` vs `data/anchorcore.db`.
- `watcher.poll` holds `Receiver` (`!Sync`) across `await` via `HashMap` iter → collect `to_sync` first.
- `cargo 1.97.1` at `$HOME/.cargo/env`, `rust/Cargo.toml:6` `1.0.8`.

## Verification (last green before overflow)
- `cargo check -p anchorcore` warnings only (70 warnings, 0 errors after `display`→`display_str` fix `main.rs:136`)
- `cargo test -p anchorcore` 35 passed before watcher addition; after watcher, overflow in `classifier::tests::provider_local_gate` (needs fix)
- Manual `curl :8128/sources` + `sync` → `done` etc. verified 2026-08-20 19:45 UTC (`/tmp/ac-sync-test`).
