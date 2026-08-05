import logging
import math
import struct

import httpx

from .app_settings import SettingsService
from .http import RetryClient

logger = logging.getLogger(__name__)

BATCH_SIZE = 32


def pack_f32(vectors: list[list[float]]) -> bytes:
    return b"".join(struct.pack("<f", v) for vector in vectors for v in vector)


def unpack_f32(data: bytes, dims: int) -> list[float]:
    return list(struct.unpack(f"<{len(data) // 4}f", data))[:dims]


class Embedder:
    def __init__(self, settings: SettingsService):
        self.settings = settings

    async def embed(self, texts: list[str]) -> list[list[float]]:
        if not texts:
            return []
        model = self.settings.get("embed_model") or "nomic-embed-text"
        base_url = self.settings.get("ollama_base_url") or "http://localhost:11434"
        timeout = self.settings.get_float("classifier_timeout", 60.0)
        vectors: list[list[float]] = []
        for start in range(0, len(texts), BATCH_SIZE):
            batch = texts[start : start + BATCH_SIZE]
            payload = {"model": model, "input": batch}
            async with RetryClient(timeout=timeout) as client:
                resp = await client.post(
                    f"{base_url.rstrip('/')}/v1/embeddings",
                    json=payload,
                )
                resp.raise_for_status()
                data = resp.json()["data"]
            vectors.extend(item["embedding"] for item in sorted(data, key=lambda d: d["index"]))
        return vectors


def cosine_similarity(a: list[float], b: list[float]) -> float:
    if not a or not b or len(a) != len(b):
        return 0.0
    dot = sum(x * y for x, y in zip(a, b))
    na = math.sqrt(sum(x * x for x in a))
    nb = math.sqrt(sum(x * x for x in b))
    if na == 0 or nb == 0:
        return 0.0
    return dot / (na * nb)
