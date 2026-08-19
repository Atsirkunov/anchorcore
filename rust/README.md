# AnchorCore Rust Port — workspace

> **Status: backlog / not started.** This folder is a *separate workspace* for an incremental Rust port. The Python backend (`/backend`) remains the shipped artifact until the Rust side passes the shared conformance suite. See `docs/rust-port.md:1` for evaluation and plan.

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

## Quick start (when implemented)

```bash
cargo build -p anchorcore
cargo run -p anchorcore -- --port 8000 --data-dir ~/.local/share/anchorcore
cargo test -p anchorcore
```

Until `crates/anchorcore` is implemented, this workspace is `cargo test` stub + backlog only.
