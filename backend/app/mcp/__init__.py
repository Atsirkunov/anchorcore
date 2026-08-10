"""B14: Model Context Protocol server for AnchorCore — thin adapters over the
backend so external harnesses (Claude Code, Codex, opencode, …) can consume
the memory with provenance.

Two transports share one tool surface:
- stdio sidecar (`backend/anchorcore_mcp.py`) — talks to a running backend via
  `BackendClient` (plain REST on 127.0.0.1; the backend stays the single owner
  of the DB and Ollama).
- streamable HTTP mounted on the FastAPI app itself (B14.2).
"""
