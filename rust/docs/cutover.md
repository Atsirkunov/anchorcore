# Cutover Checklist — R5.3 (Rust is shipped artifact) — Python retired 2026-08-21

> **Goal:** all `backend/tests` green against Rust on `:8123`, Python retired, version single-sourced in `rust/Cargo.toml` (`1.0.9` via `scripts/sync_version.py`).

## Single source of version

- **Before:** `__version__ = "1.0.8"` in `backend/app/config.py:11` was source; `rust/Cargo.toml` `workspace.package.version` was `0.1.0`.
- **After (R5.3 → 1.0.9):** `rust/Cargo.toml:6` `version = "1.0.9"` is source. Release bumps:
  1. Edit `rust/Cargo.toml` `workspace.package.version` (single edit)
  2. `cargo test -p anchorcore` + `cargo build --release` embeds `frontend/dist` via `src/frontend.rs:1`
  3. Tag `vX.Y.Z` → push → `.github/workflows/release.yml:1` builds 5 artifacts (2 Python PyInstaller + 3 Rust `AnchorCore-rust-*` with `codesign`)
  4. Python `backend/app/config.py:11` now reads `rust/Cargo.toml` at build or is kept in sync via `scripts/sync_version.py` (future: generate `backend/app/_version.py` from Cargo)

## Pre-cutover gate (must be green) — now green 2026-08-21

```bash
# 1. Rust unit + conformance
source $HOME/.cargo/env && cargo test -p anchorcore  # 47 passed (was 35 at R5.3, 44 at 1.0.9)
cargo check -p anchorcore # 0 warnings
rust/scripts/conformance.sh  # cargo test + pytest test_rust_conformance 2 passed
python rust/scripts/bench_retrieval.py --n 500 --trials 50  # vec0 p50 8-12ms vs scan 45-90ms (R0.1)

# 2. Full backend/tests vs Rust — now 110/17 honest green (was 35/121 at R5.3, 124/3 inflated, 115/8/3 at review)
ANCHOR_DATA_DIR=/tmp/ac-cutover ANCHOR_TEST_RUST_URL=http://127.0.0.1:8123 \
  cargo run -p anchorcore --bin anchorcore -- --port 8123 --data-dir /tmp/ac-cutover &
PYTHONPATH=backend backend/.venv/bin/pytest backend/tests -q \
  --ignore=backend/tests/test_mcp.py  # 110 passed / 17 skipped (127 collect)
```

**Pipeline gap closed 2026-08-21:** `POST /sources/:id/sync`/`reclassify` now `pipeline::Pipeline` folder→chunk→classify→distill→pii→embed (`src/pipeline.rs:1`), `FolderWatcher` 3s debounce `watcher/service.rs:1`, `JobManager` `BEGIN IMMEDIATE` + `PROMOTE_LOCK` atomic (+ `R10.4` queue), `Retrieval` param `IN (?,?)`/`LIKE ?` + `Hit::key` + `R10.1` history rewrite. `test_smoke`, `test_disputes`, `test_projects` etc now `110/17` honest vs Rust (was `124/3` inflated).

## Cutover steps (when gate green)

1. **Version bump:** edit `rust/Cargo.toml` + `cargo test` + `cargo build --release` + `codesign` smoke `curl /health` + `/` (frontend)
2. **Tag:** `git tag v1.0.9 && git push origin v1.0.9` — CI builds `dist/AnchorCore-rust-*` + `dist/AnchorCore-*` (Python) and attaches to GitHub Release
3. **Smoke release artifacts:** unzip `AnchorCore-rust-macos.zip` → `./AnchorCore-rust --port 8123` → `curl /health` + `curl /pii/config` + `curl /` (frontend) + `echo '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' | ./anchorcore-mcp` (MCP)
4. **Retire Python:** update `docs/packaging.md` to Rust binary as default, keep `backend/` for hosted Postgres path until `hosting/` is also ported; remove `packaging.spec` PyInstaller path from `release.yml` when Rust is sole artifact
5. **Post-cutover:** keep `backend/tests` as conformance suite via `ANCHOR_TEST_RUST_URL`; `rust/docs/decisions.md` logs any further `llama.cpp` revisit (R0.2)

## Rollback

- Re-point `frontend` `VITE_API_URL` or harness `ANCHOR_BACKEND_URL` to Python `backend` on `:8000`; Rust binary is behind `:8123` until cutover, so rollback is `pkill anchorcore-rust; ./start.sh`.

## Verification today (1.0.9 — Python retired, R10.4–R10.5 done)

- `cargo test -p anchorcore` 47 passed
- `cargo check -p anchorcore` 0 warnings
- `cargo build --release` 9.8M+3.4M `codesign valid` `target/release/anchorcore` (was 8.7M at R5.3)
- `curl` `/health` `ok` `version 1.0.9` (no `data_dir` leak, R10.8) `/system/status` `retrieval vec0` `pending 0` (no `data_dir`/`database`) `/` frontend `<!doctype>`, `/qa/search` `hits` `mcp` `tools/list` 6 tools
- `./scripts/sync_version.py --check` `ok 1.0.9` (single source `rust/Cargo.toml:6`)
- `git log --oneline -7` `52b52bd` sync fix + `0e676ae` jobs/MCP + `88a321c` db pool + `bc8a4af` watcher/SQL/RRF + `9ea85ed` R6.6 + `R10.4` queue + `R10.1` history
