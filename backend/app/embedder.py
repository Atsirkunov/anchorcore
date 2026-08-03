import logging
import math
import struct

import httpx

from .config import settings
from .http import RetryClient

logger = logging.getLogger(__name__)


def pack_f32(vectors: list[list[float]]) -> bytes:
    return b"".join(struct.pack("<f", v) for vector in vectors for v in vector)


def unpack_f32(data: bytes, dims: int) -> list[float]:
    return list(struct.unpack(f"<{len(data) // 4}f", data))[:dims]


class Embedder:
    async def embed(self, texts: list[str]) -> list[list[float]]:
        if not texts:
            return []
        payload = {"model": settings.embed_model, "input": texts}
        async with RetryClient(timeout=settings.classifier_timeout) as client:
            resp = await client.post(
                f"{settings.ollama_base_url.rstrip('/')}/v1/embeddings",
                json=payload,
            )
            resp.raise_for_status()
            data = resp.json()["data"]
        return [item["embedding"] for item in sorted(data, key=lambda d: d["index"])]


def cosine_similarity(a: list[float], b: list[float]) -> float:
    if not a or not b or len(a) != len(b):
        return 0.0
    dot = sum(x * y for x, y in zip(a, b))
    na = math.sqrt(sum(x * x for x in a))
    nb = math.sqrt(sum(x * x for x in b))
    if na == 0 or nb == 0:
        return 0.0
    return dot / (na * nb)
