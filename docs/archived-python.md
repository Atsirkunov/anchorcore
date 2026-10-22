# Archived Python backend

The Python backend left the tree and is frozen on tag
**`archive/python-final`** (92 files: `backend/`, `start.ps1`/`start.sh`,
`packaging.spec`). Main carries Rust only.

## What's frozen there

- `backend/app` — FastAPI app (incl. the old Python Team gate — superseded by `rust/crates/anchorcore/src/license.rs`, R19.1)
- `backend/alembic` — Python migrations (historical schema reference)
- `backend/tests` — the old test suite (retired 2026-10-22, R45.16; kept for archaeology only)
- `backend/requirements.txt`, `.env.example`, `run_app.py`, `anchorcore_mcp.py`

## Who consumes it

Nobody. R45.16 retired the last consumers: the CI `backend` job, the
`rust-conformance` pytest leg (19 deselects and counting), and
`rust/scripts/conformance.sh` (deleted). The API contract now lives in
`rust/crates/anchorcore/tests/api_*.rs` (black-box HTTP, part of
`cargo test`). The tag stays as history; do not restore `backend/` into
new work.

## Working with it

Read-only archaeology, e.g. `git show archive/python-final:backend/app/pipeline.py`.
Do not check `backend/` out into the tree and do not move the tag — nothing
consumes it, so a refresh buys nothing. (Never add features — the backend
is frozen.)

## Refresh log

- 2026-10-08: archived at `727a355` (main carries Rust only from here on).
- 2026-10-22: last consumers retired (R45.16); tag is history-only.
