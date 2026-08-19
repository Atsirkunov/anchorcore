# Cutover Checklist — R5.3 (Rust becomes shipped artifact)

> **Goal:** all `backend/tests` green against Rust on `:8123`, Python retires, version single-sourced in `rust/Cargo.toml`.

## Single source of version

- **Before:** `__version__ = "1.0.8"` in `backend/app/config.py:11` was source; `rust/Cargo.toml` `workspace.package.version` was `0.1.0`.
- **After (R5.3):** `rust/Cargo.toml:6` `version = "1.0.8"` is source. Release bumps:
  1. Edit `rust/Cargo.toml` `workspace.package.version` (single edit)
  2. `cargo test -p anchorcore` + `cargo build --release` embeds `frontend/dist` via `src/frontend.rs:1`
  3. Tag `vX.Y.Z` → push → `.github/workflows/release.yml:1` builds 5 artifacts (2 Python PyInstaller + 3 Rust `AnchorCore-rust-*` with `codesign`)
  4. Python `backend/app/config.py:11` now reads `rust/Cargo.toml` at build or is kept in sync via `scripts/sync_version.py` (future: generate `backend/app/_version.py` from Cargo)

## Pre-cutover gate (must be green)

```bash
# 1. Rust unit + conformance
source $HOME/.cargo/env && cargo test -p anchorcore  # 35 passed
rust/scripts/conformance.sh  # cargo test + pytest test_rust_conformance 2 passed
python rust/scripts/bench_retrieval.py --n 500 --trials 50  # vec0 p50 8-12ms vs scan 45-90ms (R0.1)

# 2. Full backend/tests vs Rust (requires pipeline wiring for /sources/:id/sync)
ANCHOR_DATA_DIR=/tmp/ac-cutover ANCHOR_TEST_RUST_URL=http://127.0.0.1:8123 \
  cargo run -p anchorcore --bin anchorcore -- --port 8123 --data-dir /tmp/ac-cutover &
PYTHONPATH=backend backend/.venv/bin/pytest backend/tests -q \
  --ignore=backend/tests/test_mcp.py  # 121 tests; currently 35/121 pass unit-only, rest need ingestion pipeline
```

**Current pipeline gap (blocks full 121 green):** `POST /sources/:id/sync` and `POST /sources/:id/reclassify` in `rust/crates/anchorcore/src/main.rs:90` are still `stubs::not_implemented` 501. `src/jobs.rs:1` + `src/scheduler.rs:1` exist, but `src/sources.rs:1` lacks sync orchestration (folder→chunk→classify→distill→pii→embed). Until this is wired, `test_smoke::test_folder_ingest_flow`, `test_disputes`, `test_projects::test_project_scopes_qa_results` etc fail against Rust (expected). R5.3 is *checklist* + version bump; pipeline wiring is next sprint.

## Cutover steps (when gate green)

1. **Version bump:** edit `rust/Cargo.toml` + `cargo test` + `cargo build --release` + `codesign` smoke `curl /health` + `/` (frontend)
2. **Tag:** `git tag v1.0.9 && git push origin v1.0.9` — CI builds `dist/AnchorCore-rust-*` + `dist/AnchorCore-*` (Python) and attaches to GitHub Release
3. **Smoke release artifacts:** unzip `AnchorCore-rust-macos.zip` → `./AnchorCore-rust --port 8123` → `curl /health` + `curl /pii/config` + `curl /` (frontend) + `echo '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' | ./anchorcore-mcp` (MCP)
4. **Retire Python:** update `docs/packaging.md` to Rust binary as default, keep `backend/` for hosted Postgres path until `hosting/` is also ported; remove `packaging.spec` PyInstaller path from `release.yml` when Rust is sole artifact
5. **Post-cutover:** keep `backend/tests` as conformance suite via `ANCHOR_TEST_RUST_URL`; `rust/docs/decisions.md` logs any further `llama.cpp` revisit (R0.2)

## Rollback

- Re-point `frontend` `VITE_API_URL` or harness `ANCHOR_BACKEND_URL` to Python `backend` on `:8000`; Rust binary is behind `:8123` until cutover, so rollback is `pkill anchorcore-rust; ./start.sh`.

## Verification today (R5.3 push)

- `cargo test -p anchorcore` 35 passed
- `cargo build --release` 8.7M+3.4M `codesign ok` `dist/AnchorCore-rust-macos.zip` 5.7M
- `cargo check --bins` ok (anchorcore + anchorcore-mcp)
- `curl` `/health` `ok` `/sources` `[]` `/system/status` `retrieval vec0` `/qa/search` `hits` `mcp` `tools/list` 6 tools
- `git log --oneline -2` shows `R5.2` + `R0.1/R0.2+R4.5`
