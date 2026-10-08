# AnchorCore Rust — agent guide (shipped `v1.0.9`)

This mirrors `/AGENTS.md` but for the `rust/` workspace. **Rust is now shipped** (`v1.0.9` `7131cfe`, `cargo test` + `cargo check` green + honest conformance vs Rust via CI `rust-conformance`). `backend/` is legacy conformance only; new work is in `rust/` unless it's `hosting/` Postgres. Read this + `rust/BACKLOG.md` + `docs/rust-port.md` + `docs/handover-2026-08-21-retire.md:1`.

## Contract-first (still, but Rust is source)

* Same REST API as `backend/app/routers/*` — same paths, same query params, same JSON shapes, same `detail` errors. The Python `backend/tests` suite (restore from tag `archive/python-final` first) is the conformance suite; `cargo test` must stay green against the Rust binary (`124/3` now green, not 35/121).
* Same SQLite file + schema: Alembic history is truth. Rust `db.rs:22` `open_db` fast path + `init_db` once (`OnceLock` migrated), `PRAGMA foreign_keys=ON` stays (`backend/app/db.py`), `WAL`/`busy_timeout` 5s. Never hold `Connection` (`!Send`) across `await` — `spawn_blocking` like `sources.rs:121`.
* Frontend unchanged: `frontend/dist` is embedded via `include_dir!` (`frontend.rs:27`) in Rust binary (`9.8M` + `3.4M` `codesign valid`).

## How to work here (now tech-debt, not greenfield)

* Pick one task from `rust/BACKLOG.md` **Phase 7–10** (`R7.1`–`R10.9`, release-blocking security/retrieval/conformance first) or the `P*` remainder in `docs/port-review-2026-08-21-lead.md:44` (expand_context, logs pagination, deadpool, etc.) — tasks still sized for parallel agents (no shared files). Full review driving these: `docs/review-2026-08-21-full.md:1`.
* For each task: schema/migration (if any) → router/handler → wire in `main.rs` → test (new `cargo test` green + existing Python `backend/tests` via HTTP on `:8123`) → docs. Note: R9.2 makes conformance honest (`127` collect → `110/17` vs Rust via CI `rust-conformance`); old "124/3" & "115/8/3" were inflated/stale (see `docs/review-2026-08-21-full.md:1`).
* Keep `rust/Cargo.toml:6` single source (`1.0.9` via `scripts/sync_version.py`); crates live under `rust/crates/*`.
* Log decisions in `rust/docs/decisions.md`; update `docs/handover-*.md` on retire.

## Gotchas (from Python)

* Job concurrency bounded (`JobManager.MAX_CONCURRENT=2`) — keep it.
* Retrieval must filter `stale`/`disputed` via `_status_ok` equivalent — every path must gate it.
* Secrets via OS keychain (`keyring` crate) + `***set***` placeholder — same UX as `backend/app/secrets.py`.
* `vec0` vs Python scan fallback — `rust` should use `sqlite-vec` statically linked.

## Cyclomatic complexity (Phase 13 — design for it from day one)

* **Design rule: keep every function ≤ 15 CCN** (lizard). When a handler/orchestrator grows past ~12, split it into named phase helpers (e.g. `pipeline.rs` `ClassifyCtx` phases, `system.rs` `status_snapshot`, `watcher/service.rs` `fetch_and_compare`) instead of nesting `if let`/`match` chains. Prefer guard clauses (`let Some(x) = y else { return ... }`) over nested conditionals.
* **Acceptance gate:** `lizard -l rust -w rust/crates/anchorcore/src -C 15` must print no NEW warnings beyond the accepted `is_public_path` (auth.rs decision table — explicitly do-not-touch). Run it alongside `cargo test` + `cargo check` on every task.
* Reusable patterns from R13: `with_db`/`run_db` in `pipeline.rs` for all `spawn_blocking` DB work; row-mapper helper fns (e.g. `error_row`, `hit_from_row10`) instead of inline closures; dynamic `params_from_iter` vecs instead of 4× match arms.
* See `rust/BACKLOG.md` Phase 13 + `rust/docs/decisions.md` 2026-08-29 entry for the full rationale.
