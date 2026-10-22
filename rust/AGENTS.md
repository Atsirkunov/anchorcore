# AnchorCore Rust — agent guide (shipped `v1.0.20`)

This mirrors `/AGENTS.md` but for the `rust/` workspace. **Rust is shipped** (`cargo test` + `cargo check` green; the API contract tests in `tests/api_*.rs` run inside `cargo test`). `backend/` is archived history (R45.16 retired the last consumers); new work is in `rust/` — `hosting/` is this same binary in one container (R19.1, license gate `license.rs`). Read this + `rust/BACKLOG.md` + `docs/archive/rust-port.md` + `docs/archive/handover-2026-08-21-retire.md:1`.

## Contract-first (still, but Rust is source)

* Same REST API as the archived `backend/app/routers/*` — same paths, same query params, same JSON shapes, same `detail` errors. The contract is pinned by `tests/api_*.rs` (black-box HTTP: each test spawns the binary on a free port with a fresh temp data dir — no shared DB, no serial lock); extend them when the API changes.
* Same SQLite file + schema: Alembic history is truth. Rust `db.rs:22` `open_db` fast path + `init_db` once (`OnceLock` migrated), `PRAGMA foreign_keys=ON` stays (`backend/app/db.py`), `WAL`/`busy_timeout` 5s. Never hold `Connection` (`!Send`) across `await` — `spawn_blocking` like `sources.rs:121`.
* Frontend unchanged: `frontend/dist` is embedded via `include_dir!` (`frontend.rs:27`) in Rust binary (`9.8M` + `3.4M` `codesign valid`).

## How to work here (now tech-debt, not greenfield)

* Pick open work from `rust/BACKLOG.md`: **Phase 20** (`R20.*`, MCP zero-setup UX — `docs/plan-mcp-zero-setup.md`), **Phase 23** (`R23.*`, agent-friendly config), **Phase 18** (`R18.*`, website audit) — plus the shipped log (R19, R21–R24). Completed Phases 0–17: `rust/BACKLOG-archive.md`. Tasks still sized for parallel agents (file-disjoint). Full review driving earlier phases: `docs/archive/review-2026-08-21-full.md:1`.
* For each task: schema/migration (if any) → router/handler → wire in `main.rs` → test (`cargo test` green, incl. the `tests/api_*.rs` contract tests) → docs.
* Keep `rust/Cargo.toml:6` single source (`1.0.20` via `scripts/sync_version.py`); crates live under `rust/crates/*`.
* Log decisions in `rust/docs/decisions.md`; update `docs/archive/handover-*.md` on retire.

## Gotchas (from Python)

* Job concurrency bounded (`JobManager.MAX_CONCURRENT=2`) — keep it.
* Retrieval must filter `stale`/`disputed` via `_status_ok` equivalent — every path must gate it.
* Secrets via OS keychain (`keyring` crate) + `***set***` placeholder — same UX as `backend/app/secrets.py`.
* **Toolchain on this box (2026-10-12): Rust builds work.** No MSVC, but a portable MinGW + GNU-host toolchain is installed and cached: `C:\Users\Aleksandr\.local\winlibs\mingw64\bin` (WinLibs 16.2, ucrt) + rustup `stable-x86_64-pc-windows-gnu`. Invoke:
  `$env:PATH = "C:\Users\Aleksandr\.local\winlibs\mingw64\bin;" + $env:PATH; cargo +stable-x86_64-pc-windows-gnu test --offline -p anchorcore --target x86_64-pc-windows-gnu` (from `rust/`). `cargo check`/`test`/`clippy`/`build` all work; `cargo +stable-x86_64-pc-windows-msvc` still fails (`link.exe` absent). `rust/target` is shared with the main checkout — a fresh `git worktree` needs `robocopy` of `rust/target` and `frontend/dist` (both gitignored) or a full rebuild + `npm run build` (see `docs/archive/handover-2026-10-10-b54-verify.md`).
* CI runs the real `cargo test`/clippy on every push (`ci.yml` `rust` job); keep them green locally too.
* No `sqlite-vec` is linked: `vec_chunks` is a plain table and retrieval runs the embedding scan. Migration 21 (R45.0) drops Python-era vec0 indexes; linking sqlite-vec is R43.10.

## Cyclomatic complexity (Phase 13 — design for it from day one)

* **Design rule: keep every function ≤ 15 CCN** (lizard). When a handler/orchestrator grows past ~12, split it into named phase helpers (e.g. `pipeline.rs` `ClassifyCtx` phases, `system.rs` `status_snapshot`, `watcher/service.rs` `fetch_and_compare`) instead of nesting `if let`/`match` chains. Prefer guard clauses (`let Some(x) = y else { return ... }`) over nested conditionals.
* **Acceptance gate:** `lizard -l rust -w rust/crates/anchorcore/src -C 15` must print no NEW warnings beyond the accepted baseline (7 as of 1.0.33 — `rtf_to_text` 31 [filed R36.14], `create_summaries_inner` 32, `create_tags_inner` 19, `conflicts_for_chunks`/`load_hits_filtered`/`load_chunks_with_embedding_filtered` 17, plus the `text_decodes_*` test-module tokenizer artifact; `is_public_path` 22 retired by the R43.8 allowlist rewrite, `classify_windows` 18 resolved). Run it alongside `cargo test` + `cargo check` on every task; if your diff adds a warning, split the function.
* Reusable patterns from R13: `with_db`/`run_db` in `pipeline.rs` for all `spawn_blocking` DB work; row-mapper helper fns (e.g. `error_row`, `hit_from_row10`) instead of inline closures; dynamic `params_from_iter` vecs instead of 4× match arms.
* See `rust/BACKLOG-archive.md` Phase 13 + `rust/docs/decisions.md` 2026-08-29 entry for the full rationale.
