# Archived Python backend

The Python backend left the tree and is frozen on tag
**`archive/python-final`** (92 files: `backend/`, `start.ps1`/`start.sh`,
`packaging.spec`). Main carries Rust only.

## What's frozen there

- `backend/app` — FastAPI app (incl. the old Python Team gate — superseded by `rust/crates/anchorcore/src/license.rs`, R19.1)
- `backend/alembic` — Python migrations (historical schema reference)
- `backend/tests` — test suite incl. the Rust conformance suite
- `backend/requirements.txt`, `.env.example`, `run_app.py`, `anchorcore_mcp.py`

## Who consumes it

- Nothing in `hosting/` anymore — R19.1 builds the Rust binary from main.
- CI (`backend`, `rust-conformance` jobs) checks the tag out.
- `rust/scripts/conformance.sh` restores it automatically when missing.

## Working with it

Restore into your checkout (does not move HEAD):

```bash
git checkout archive/python-final -- backend
```

To change frozen code: commit on a branch, move the tag, re-run the conformance suite, record it below. (Never add features — the backend is frozen.)

Pinning decision (B46): consumers track the tag by name, not a commit
SHA. Rationale: this repo is solo-controlled — anyone who can move the tag
can push main. Revisit SHA pinning if outside contributors join.

## Refresh log

- 2026-10-08: archived at `727a355` (main carries Rust only from here on).
