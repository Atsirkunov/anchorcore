# Archived Python backend

The Python backend left the tree and is frozen on tag
**`archive/python-final`** (92 files: `backend/`, `start.ps1`/`start.sh`,
`packaging.spec`). Main carries Rust only.

## What's frozen there

- `backend/app` — FastAPI app incl. `license.py` (Team license gate)
- `backend/alembic` — migrations (run by the hosting image)
- `backend/tests` — test suite incl. the Rust conformance suite
- `backend/requirements.txt`, `.env.example`, `run_app.py`, `anchorcore_mcp.py`

## Who consumes it

- `hosting/Dockerfile` downloads the tag tarball at build time
  (`ARG BACKEND_TAG`, default `archive/python-final`).
- CI (`backend`, `rust-conformance` jobs) checks the tag out.
- `rust/scripts/conformance.sh` restores it automatically when missing.

## Working with it

Restore into your checkout (does not move HEAD):

```bash
git checkout archive/python-final -- backend
```

To change frozen code (license-gate fixes only — never features): commit on a
branch, move the tag, rebuild + re-verify the Team image, record it here.

## Refresh log

- 2026-10-08: archived at `727a355` (main carries Rust only from here on).
