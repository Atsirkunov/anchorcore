"""AnchorCore MCP sidecar (stdio transport).

A separate process spawned by a harness (Claude Code, Codex, opencode, …) that
connects to the *running* AnchorCore backend over 127.0.0.1 — the backend stays
the single owner of the DB and Ollama.

Usage:
    python anchorcore_mcp.py                # backend at http://127.0.0.1:8000
    ANCHOR_BACKEND_URL=http://127.0.0.1:8123 python anchorcore_mcp.py
    ANCHOR_MCP_TOKEN=... python anchorcore_mcp.py   # bearer token for centralized backends

Harness setup (local, backend already running):
    claude mcp add anchorcore -- python /path/to/backend/anchorcore_mcp.py
    codex mcp add anchorcore -- python /path/to/backend/anchorcore_mcp.py
"""

import asyncio
import os
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from app.mcp.backend import HttpBackend  # noqa: E402
from app.mcp.server import build_server  # noqa: E402

DEFAULT_URL = os.environ.get("ANCHOR_BACKEND_URL", "http://127.0.0.1:8000")
TOKEN = os.environ.get("ANCHOR_MCP_TOKEN", "")


async def main() -> None:
    backend = HttpBackend(DEFAULT_URL, token=TOKEN)
    mcp = build_server(backend)
    await mcp.run_stdio_async()


if __name__ == "__main__":
    asyncio.run(main())
