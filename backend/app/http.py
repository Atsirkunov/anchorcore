import asyncio
import logging
import random

import httpx

from .config import settings

logger = logging.getLogger(__name__)

RETRYABLE_STATUS = {429, 500, 502, 503, 504}
BASE_BACKOFF = 1.0


def _backoff(attempt: int) -> float:
    return BASE_BACKOFF * (2**attempt) + random.uniform(0, 0.5)


class RetryClient:
    """Async httpx client with exponential backoff for transient failures."""

    def __init__(self, timeout: float, max_retries: int | None = None, **kwargs):
        timeout_obj = httpx.Timeout(timeout=timeout, connect=5.0)
        self._client = httpx.AsyncClient(timeout=timeout_obj, **kwargs)
        self.max_retries = settings.http_retries if max_retries is None else max_retries

    async def __aenter__(self) -> "RetryClient":
        return self

    async def __aexit__(self, *_exc) -> None:
        await self._client.aclose()

    async def get(self, url: str, **kwargs) -> httpx.Response:
        return await self._run("GET", url, **kwargs)

    async def post(self, url: str, **kwargs) -> httpx.Response:
        return await self._run("POST", url, **kwargs)

    async def _run(self, method: str, url: str, **kwargs) -> httpx.Response:
        last_exc: Exception | None = None
        for attempt in range(self.max_retries + 1):
            try:
                resp = await self._client.request(method, url, **kwargs)
                if resp.status_code in RETRYABLE_STATUS and attempt < self.max_retries:
                    logger.warning("%s %s -> %s, retrying (%d)", method, url, resp.status_code, attempt + 1)
                    await asyncio.sleep(_backoff(attempt))
                    continue
                return resp
            except httpx.TransportError as exc:
                last_exc = exc
                if attempt < self.max_retries:
                    logger.warning("%s %s failed (%s), retrying (%d)", method, url, type(exc).__name__, attempt + 1)
                    await asyncio.sleep(_backoff(attempt))
                    continue
                raise
        assert last_exc is not None
        raise last_exc
