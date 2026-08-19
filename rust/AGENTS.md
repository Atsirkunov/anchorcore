# AnchorCore Rust — agent guide

This mirrors `/AGENTS.md` but for the `rust/` workspace. AI agents working on the port should read this + `rust/BACKLOG.md` + `docs/rust-port.md`.

## Contract-first

* Same REST API as `backend/app/routers/*` — same paths, same query params, same JSON shapes, same `detail` errors. The Python `backend/tests` suite is the conformance suite; `cargo test` must stay green against the Rust binary.
* Same SQLite file + schema: Alembic history is truth. Rust applies migrations via `rusqlite_migration`/`refinery`, never invents tables. `PRAGMA foreign_keys=ON` stays (`backend/app/db.py`).
* Frontend unchanged: `frontend/dist` is embedded in both binaries.

## How to work here

* Pick one task from `rust/BACKLOG.md` — tasks are sized for parallel agents (no shared files).
* For each task: schema/migration (if any) → router/handler → wire in `main.rs` → test (new `cargo test` + existing Python `backend/tests` via HTTP) → docs.
* Keep `rust/Cargo.toml` workspace root; crates live under `rust/crates/*`.
* Log decisions in `rust/docs/decisions.md`.

## Gotchas (from Python)

* Job concurrency bounded (`JobManager.MAX_CONCURRENT=2`) — keep it.
* Retrieval must filter `stale`/`disputed` via `_status_ok` equivalent — every path must gate it.
* Secrets via OS keychain (`keyring` crate) + `***set***` placeholder — same UX as `backend/app/secrets.py`.
* `vec0` vs Python scan fallback — `rust` should use `sqlite-vec` statically linked.
