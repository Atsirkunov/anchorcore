"""MCP tool implementations — thin adapters over the backend.

Every tool is a plain async function of (backend, args) so tests can drive
them directly; `server.py` registers them on a FastMCP instance. Results are
JSON-serializable dicts; retrieval results respect the backend's status
filtering (stale/disputed excluded) — agents never see contested facts.
"""

from typing import Any

from .backend import Backend, BackendError

MAX_CONTENT = 4000


def _clip(text: str, limit: int = MAX_CONTENT) -> str:
    return (text or "")[:limit]


async def tool_ask(backend: Backend, question: str, project_id: int | None = None) -> dict[str, Any]:
    """Answer a question with citations into the memory's sources.

    Uses the full AnchorCore pipeline (hybrid retrieval + graph expansion +
    LLM synthesis when a model is configured). Returns `answer` plus
    `citations`, each with `source_ref` (file/section), kind, confidence
    score, and a snippet — so the harness can verify provenance itself.
    """
    if not question or not question.strip():
        raise BackendError("question must be a non-empty string")
    result = await backend.ask(question.strip(), project_id=project_id)
    return {
        "answer": result.get("answer", ""),
        "citations": [
            {
                "entity_id": c.get("entity_id"),
                "kind": c.get("kind", "document"),
                "summary": _clip(c.get("summary", ""), 500),
                "source_ref": c.get("source_ref", ""),
                "score": c.get("score"),
                "snippet": _clip(c.get("snippet", ""), 500),
            }
            for c in result.get("citations", [])
        ],
    }


async def tool_search(backend: Backend, query: str, k: int = 8, project_id: int | None = None) -> dict[str, Any]:
    """Raw retrieval: ranked context chunks/entities for the query.

    The workhorse for harnesses — returns evidence (content, source_ref,
    entity kind/summary, score) to feed the agent's own reasoning loop,
    instead of a final answer it can't inspect. Respects status filtering
    (stale/disputed excluded) and `project_id` scoping.
    """
    if not query or not query.strip():
        raise BackendError("query must be a non-empty string")
    if k < 1 or k > 50:
        raise BackendError("k must be between 1 and 50")
    result = await backend.search(query.strip(), k=int(k), project_id=project_id)
    hits = []
    for hit in result.get("hits", []):
        hits.append(
            {
                "chunk_id": hit.get("chunk_id"),
                "entity_id": hit.get("entity_id"),
                "kind": hit.get("kind", "document"),
                "summary": _clip(hit.get("summary", ""), 500),
                "content": _clip(hit.get("content", "")),
                "source_ref": hit.get("source_ref", ""),
                "source_id": hit.get("source_id"),
                "item_id": hit.get("item_id"),
                "item_title": hit.get("item_title", ""),
                "score": hit.get("score"),
            }
        )
    return {"query": query, "hits": hits}


async def tool_get_entity(backend: Backend, entity_id: int) -> dict[str, Any]:
    """Fetch a single entity with its provenance: summary, kind, confidence,
    status, `window_text` (exactly what the classifier saw), source_ref, and
    dispute count."""
    entity = await backend.get_entity(int(entity_id))
    if entity is None:
        raise BackendError(f"entity {entity_id} not found")
    return {
        "id": entity["id"],
        "kind": entity.get("kind", ""),
        "summary": _clip(entity.get("summary", ""), 1000),
        "reasoning": _clip(entity.get("reasoning", ""), 1000),
        "confidence": entity.get("confidence"),
        "status": entity.get("status", ""),
        "author": entity.get("author", ""),
        "owner": entity.get("owner", ""),
        "source_ref": entity.get("source_ref", ""),
        "window_text": _clip(entity.get("window_text", ""), MAX_CONTENT),
        "dispute_count": entity.get("dispute_count", 0),
        "created_at": entity.get("created_at"),
    }


async def tool_get_source(backend: Backend, source_id: int) -> dict[str, Any]:
    """Fetch a source's details (name, connector, enabled, last sync, config).
    Secret config values are masked — never shipped to the harness."""
    source = await backend.get_source(int(source_id))
    if source is None:
        raise BackendError(f"source {source_id} not found")
    return source


async def tool_list_sources(backend: Backend) -> dict[str, Any]:
    """List all connected sources: name, connector, enabled, last sync, error
    state — a map of what the memory can answer about."""
    sources = await backend.list_sources()
    return {
        "sources": [
            {
                "id": s["id"],
                "name": s.get("name", ""),
                "connector": s.get("connector", ""),
                "enabled": s.get("enabled", True),
                "last_synced_at": s.get("last_synced_at"),
                "last_error": _clip(s.get("last_error", "") or "", 300),
                "error_count": s.get("error_count", 0),
            }
            for s in sources
        ]
    }


async def tool_memory_status(backend: Backend) -> dict[str, Any]:
    """Memory health: version, model availability, pending embeddings, and
    scheduler state — so a harness can tell whether answers will be degraded
    (no Ollama/models) before relying on them."""
    return await backend.memory_status()
