"""Assemble the MCP server from shared tool implementations.

`build_server(backend)` returns a FastMCP instance usable with either
transport:
- stdio: the sidecar (`backend/anchorcore_mcp.py`) calls
  `await mcp.run_stdio_async()`.
- streamable HTTP (B14.2): mount `mcp.streamable_http_app(...)` on the
  FastAPI app with an in-process backend.
"""

import logging

from mcp.server.fastmcp import FastMCP

from . import tools
from .backend import Backend

logger = logging.getLogger(__name__)

TOOL_DESCRIPTIONS = {
    "ask": "Answer a question using AnchorCore memory, with citations into sources.",
    "search": "Ranked raw retrieval (chunks/entities + provenance + scores) for a query.",
    "get_entity": "Fetch one entity with provenance (summary, confidence, status, window_text, source_ref).",
    "get_source": "Fetch one source's details and config (secrets masked).",
    "list_sources": "List all connected sources.",
    "memory_status": "Memory health: version, model availability, pending embeddings.",
}


def build_server(backend: Backend, name: str = "anchorcore") -> FastMCP:
    mcp = FastMCP(name=name)

    @mcp.tool(description=TOOL_DESCRIPTIONS["ask"])
    async def ask(question: str, project_id: int | None = None) -> dict:
        return await tools.tool_ask(backend, question, project_id)

    @mcp.tool(description=TOOL_DESCRIPTIONS["search"])
    async def search(query: str, k: int = 8, project_id: int | None = None) -> dict:
        return await tools.tool_search(backend, query, k, project_id)

    @mcp.tool(description=TOOL_DESCRIPTIONS["get_entity"])
    async def get_entity(entity_id: int) -> dict:
        return await tools.tool_get_entity(backend, entity_id)

    @mcp.tool(description=TOOL_DESCRIPTIONS["get_source"])
    async def get_source(source_id: int) -> dict:
        return await tools.tool_get_source(backend, source_id)

    @mcp.tool(description=TOOL_DESCRIPTIONS["list_sources"])
    async def list_sources() -> dict:
        return await tools.tool_list_sources(backend)

    @mcp.tool(description=TOOL_DESCRIPTIONS["memory_status"])
    async def memory_status() -> dict:
        return await tools.tool_memory_status(backend)

    return mcp
