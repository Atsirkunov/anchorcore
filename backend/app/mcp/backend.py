"""Backend adapter for the MCP server.

The stdio sidecar is a separate process from the backend; it must NOT open the
SQLite DB itself (single-writer rule) — it talks REST to the running backend on
127.0.0.1, exactly like the browser UI does. The HTTP transport (B14.2) instead
provides an in-process implementation with the same method surface.
"""

import logging
from typing import Any, Protocol

import httpx

logger = logging.getLogger(__name__)


class BackendError(RuntimeError):
    """The backend is unreachable or returned an error."""


class Backend(Protocol):
    """Method surface the MCP tools need. Implemented over HTTP (sidecar) or
    in-process (HTTP transport mounted on the app)."""

    async def ask(self, question: str, project_id: int | None = None) -> dict[str, Any]: ...

    async def search(self, query: str, k: int = 8, project_id: int | None = None) -> dict[str, Any]: ...

    async def get_entity(self, entity_id: int) -> dict[str, Any] | None: ...

    async def get_source(self, source_id: int) -> dict[str, Any] | None: ...

    async def list_sources(self) -> list[dict[str, Any]]: ...

    async def memory_status(self) -> dict[str, Any]: ...


class HttpBackend:
    """Thin HTTP adapter over the AnchorCore REST API (127.0.0.1 by default).

    `token` is sent as a Bearer header when set (centralized deployments with
    `ANCHOR_MCP_TOKEN`); plain localhost needs no token.
    """

    def __init__(self, base_url: str, token: str = "", timeout: float = 60.0, transport=None):
        self.base_url = base_url.rstrip("/")
        self.token = token
        self.timeout = timeout
        self.transport = transport  # httpx transport override (tests: ASGITransport)

    async def _call(self, method: str, path: str, json: dict | None = None) -> Any:
        headers = {}
        if self.token:
            headers["Authorization"] = f"Bearer {self.token}"
        try:
            kwargs = {"base_url": self.base_url, "timeout": self.timeout}
            if self.transport is not None:
                kwargs["transport"] = self.transport
            async with httpx.AsyncClient(**kwargs) as client:
                resp = await client.request(method, path, json=json, headers=headers)
        except httpx.HTTPError as exc:
            raise BackendError(
                f"AnchorCore backend unreachable at {self.base_url} ({type(exc).__name__}). "
                "Is the app running?"
            ) from exc
        if resp.status_code == 404:
            return None
        if resp.status_code >= 400:
            detail = resp.text[:500]
            raise BackendError(f"backend {method} {path} failed ({resp.status_code}): {detail}")
        return resp.json()

    async def ask(self, question: str, project_id: int | None = None) -> dict[str, Any]:
        return await self._call("POST", "/qa", {"question": question, "project_id": project_id})

    async def search(self, query: str, k: int = 8, project_id: int | None = None) -> dict[str, Any]:
        return await self._call(
            "POST", "/qa/search", {"query": query, "k": k, "project_id": project_id}
        )

    async def get_entity(self, entity_id: int) -> dict[str, Any] | None:
        return await self._call("GET", f"/entities/{entity_id}")

    async def get_source(self, source_id: int) -> dict[str, Any] | None:
        return await self._call("GET", f"/sources/{source_id}")

    async def list_sources(self) -> list[dict[str, Any]]:
        return await self._call("GET", "/sources")

    async def memory_status(self) -> dict[str, Any]:
        return await self._call("GET", "/system/status")
