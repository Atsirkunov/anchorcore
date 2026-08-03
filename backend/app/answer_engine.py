import json
import logging

import httpx
from sqlalchemy import select
from sqlalchemy.orm import Session

from .config import settings
from .embedder import Embedder, unpack_f32
from .http import RetryClient
from .models import Chunk, Entity
from .schemas import AskResponse, Citation

logger = logging.getLogger(__name__)

SYSTEM_PROMPT = """You are AnchorCore, the memory of an organization.
Answer the user's question using ONLY the provided context sections.
Each section is tagged with a section ID like [S1], [S2]...
Always cite the sections you use by ID at the end of the relevant sentence.
If the context does not contain the answer, say so clearly instead of guessing.
"""


class AnswerEngine:
    def __init__(self, embedder: Embedder):
        self.embedder = embedder

    async def ask(self, db: Session, question: str) -> AskResponse:
        try:
            query_embedding = (await self.embedder.embed([question]))[0]
            hits = self._vector_search(db, query_embedding)
        except Exception as exc:  # noqa: BLE001
            logger.warning("embedding/search failed (%s); using keyword search", exc)
            hits = self._keyword_search(db)

        if not hits:
            return AskResponse(
                answer="No relevant knowledge found yet. Ingest sources first.",
                citations=[],
            )

        sections = []
        for idx, hit in enumerate(hits, start=1):
            content = hit["chunk"].content[:2000]
            sections.append(f"[S{idx}] {content}")
        context = "\n\n".join(sections)

        answer_text = await self._generate(question, context)

        citations = [
            Citation(
                entity_id=hit["chunk"].entity_id,
                kind=hit["entity"].kind,
                summary=hit["entity"].summary[:200],
                source_ref=hit["chunk"].source_ref,
                score=round(hit["score"], 3),
                snippet=hit["chunk"].content[:300],
            )
            for hit in hits[:5]
        ]
        return AskResponse(answer=answer_text, citations=citations)

    def _vector_search(self, db: Session, query_embedding: list[float]) -> list[dict]:
        rows = db.execute(
            select(Chunk, Entity)
            .join(Entity, Chunk.entity_id == Entity.id)
            .where(Chunk.embedding.is_not(None), Entity.status != "stale")
        ).all()

        scored = []
        for chunk, entity in rows:
            vector = unpack_f32(chunk.embedding, len(query_embedding))
            score = _cosine(query_embedding, vector)
            scored.append({"chunk": chunk, "entity": entity, "score": score})

        scored.sort(key=lambda r: r["score"], reverse=True)
        return [hit for hit in scored[: settings.top_k] if hit["score"] > 0.2]

    def _keyword_search(self, db: Session) -> list[dict]:
        rows = db.execute(
            select(Chunk, Entity)
            .join(Entity, Chunk.entity_id == Entity.id)
            .where(Entity.status != "stale")
            .order_by(Entity.created_at.desc())
            .limit(settings.top_k)
        ).all()
        return [{"chunk": chunk, "entity": entity, "score": 0.0} for chunk, entity in rows]

    async def _generate(self, question: str, context: str) -> str:
        if settings.answer_base_url.startswith("http://localhost") or settings.answer_base_url.startswith("http://127.0.0.1"):
            return f"Answer for: {question}\n\n[Note: answer model unavailable offline; showing top context.]\n\n{context[:1500]}"

        if not settings.answer_api_key:
            return f"Answer for: {question}\n\n[No model key configured. Matching context:]\n\n{context[:1500]}"

        payload = {
            "model": settings.answer_model,
            "messages": [
                {"role": "system", "content": SYSTEM_PROMPT},
                {"role": "user", "content": f"Question: {question}\n\nContext:\n{context}"},
            ],
            "temperature": 0.2,
        }
        async with RetryClient(timeout=settings.answer_timeout) as client:
            resp = await client.post(
                f"{settings.answer_base_url.rstrip('/')}/chat/completions",
                headers={"Authorization": f"Bearer {settings.answer_api_key}"},
                json=payload,
            )
            resp.raise_for_status()
            return resp.json()["choices"][0]["message"]["content"]


def _cosine(a: list[float], b: list[float]) -> float:
    import math

    if not a or not b or len(a) != len(b):
        return 0.0
    dot = sum(x * y for x, y in zip(a, b))
    na = math.sqrt(sum(x * x for x in a))
    nb = math.sqrt(sum(x * x for x in b))
    if na == 0 or nb == 0:
        return 0.0
    return dot / (na * nb)
