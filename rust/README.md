# AnchorCore Rust Port — workspace

> **Status: shipped as of `v1.0.9` `7131cfe` (2026-08-21).** Python `backend/` is now legacy conformance + hosted Postgres (see R10.7: `hosting/` stays Python); Rust `rust/` is the shipped artifact (single source `rust/Cargo.toml:6` → `backend/app/config.py:11` via `scripts/sync_version.py`). All `backend/tests` `110/17` honest green vs Rust on `:8123` (`cargo test` + `cargo check` green via CI `rust-conformance`). See `docs/rust-port.md:1` (evaluation closed) + `rust/BACKLOG.md:1` (all R0–R6 done, R7–R10.6 done) + `docs/handover-2026-08-21-retire.md:1`.

## Why a separate folder?

* AI agents can work here in parallel without touching Python code
* Same REST API + same SQLite file = contract, both sides stay green
* Frontend (`/frontend`) is untouched — it talks to `localhost:8000` regardless of language

## Layout

```
rust/
  README.md          — this file
  BACKLOG.md         — AI-agent friendly task backlog (the source of truth for the port)
  AGENTS.md          — agent guide for this workspace (mirrors /AGENTS.md but Rust-specific)
  Cargo.toml         — workspace root
  crates/
    anchorcore/      — axum + rusqlite + sqlite-vec binary
      Cargo.toml
      src/
        main.rs      — entry (mirrors backend/run_app.py)
        lib.rs
  docs/
    decisions.md     — ADR log
```

## Contract

* **API:** must be byte-for-byte compatible with `backend/app/routers/*` (same routes, same JSON, same error shapes). The `backend/tests` suite is the conformance suite.
* **DB:** reads/writes the *same* SQLite file (Alembic migrations are source of truth; Rust uses `rusqlite` + `refinery`/`rusqlite_migration` to apply them, never invents schema).
* **Secrets:** OS keychain via `keyring` crate, same keys as `backend/app/secrets.py:1`.
* **Frontend:** embedded via `include_dir!` (like `packaging.spec` does for `frontend/dist`).

## Quick start (shipped)

```bash
source $HOME/.cargo/env
cargo build -p anchorcore --release  # 9.8M anchorcore + 3.4M anchorcore-mcp, codesign valid
cargo run -p anchorcore -- --port 8123 --data-dir /tmp/ac-data & curl http://127.0.0.1:8123/health
cargo test -p anchorcore  # all green (harness + jobs atomic + auth 401 + scheduler + R10.4 queue)
cargo check -p anchorcore # 0 warnings
scripts/sync_version.py --check # ok 1.0.9 (single source)
PYTHONPATH=backend ANCHOR_TEST_RUST_URL=http://127.0.0.1:8123 pytest -q --ignore=test_mcp.py # honest accounting vs Rust
```

Release is `rust/Cargo.toml:6` single edit → `scripts/sync_version.py` → `cargo test` + `cargo build --release` → `git tag vX.Y.Z` → `git push origin vX.Y.Z` (CI `release.yml` builds `AnchorCore-{windows,macos,linux}.zip`).

Petite note: `backend/` remains for `hosting/` Postgres path; `packaging.spec` PyInstaller retired.
