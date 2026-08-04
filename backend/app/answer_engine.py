import json
import logging

from sqlalchemy import or_, select
from sqlalchemy.orm import Session

from .config import settings
from .embedder import Embedder, unpack_f32
from .http import RetryClient
from .models import Chunk, Entity, IngestedItem
from .schemas import AskResponse, Citation
from .system_events import record as record_event

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
            record_event(
                "qa",
                "retrieval degraded to newest-chunks fallback (embedding/search failed)",
                level="warning",
                detail=f"{type(exc).__name__}: {exc}",
            )
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

        citations = []
        for hit in hits[:5]:
            chunk, entity, item = hit["chunk"], hit["entity"], hit["item"]
            if entity is not None:
                kind, summary = entity.kind, entity.summary[:200]
            else:
                kind = "document"
                summary = (item.title if item is not None else chunk.source_ref)[:200]
            citations.append(
                Citation(
                    entity_id=entity.id if entity is not None else None,
                    kind=kind,
                    summary=summary,
                    source_ref=chunk.source_ref,
                    score=round(hit["score"], 3),
                    snippet=chunk.content[:300],
                )
            )
        return AskResponse(answer=answer_text, citations=citations)

    def _vector_search(self, db: Session, query_embedding: list[float]) -> list[dict]:
        rows = db.execute(self._chunk_query().where(Chunk.embedding.is_not(None))).all()

        scored = []
        for chunk, entity, item in rows:
            if entity is not None and entity.status == "stale":
                continue
            vector = unpack_f32(chunk.embedding, len(query_embedding))
            score = _cosine(query_embedding, vector)
            scored.append({"chunk": chunk, "entity": entity, "item": item, "score": score})

        scored.sort(key=lambda r: r["score"], reverse=True)
        return [hit for hit in scored[: settings.top_k] if hit["score"] > 0.2]

    def _keyword_search(self, db: Session) -> list[dict]:
        rows = db.execute(
            self._chunk_query().order_by(Chunk.created_at.desc()).limit(settings.top_k)
        ).all()
        return [
            {"chunk": chunk, "entity": entity, "item": item, "score": 0.0}
            for chunk, entity, item in rows
            if entity is None or entity.status != "stale"
        ]

    def _chunk_query(self):
        """Chunks of type: entity-summary (entity set) or full-document (item set)."""
        return (
            select(Chunk, Entity, IngestedItem)
            .outerjoin(Entity, Chunk.entity_id == Entity.id)
            .outerjoin(IngestedItem, Chunk.item_id == IngestedItem.id)
        )

    async def _generate(self, question: str, context: str) -> str:
        is_local = settings.answer_base_url.startswith(("http://localhost", "http://127.0.0.1"))
        if not is_local and not settings.answer_api_key:
            return f"Answer for: {question}\n\n[No model key configured. Matching context:]\n\n{context[:1500]}"

        headers = {"Authorization": f"Bearer {settings.answer_api_key}"} if settings.answer_api_key else {}
        payload = {
            "model": settings.answer_model,
            "messages": [
                {"role": "system", "content": SYSTEM_PROMPT},
                {"role": "user", "content": f"Question: {question}\n\nContext:\n{context}"},
            ],
            "temperature": 0.2,
        }
        try:
            async with RetryClient(timeout=settings.answer_timeout) as client:
                resp = await client.post(
                    f"{settings.answer_base_url.rstrip('/')}/chat/completions",
                    headers=headers,
                    json=payload,
                )
                resp.raise_for_status()
                return resp.json()["choices"][0]["message"]["content"]
        except Exception as exc:  # noqa: BLE001
            logger.warning("answer generation failed (%s); returning context only", exc)
            record_event(
                "qa",
                "answer generation failed; returning matching context only",
                level="warning",
                detail=f"{type(exc).__name__}: {exc}",
            )
            return f"Answer for: {question}\n\n[Answer model unreachable ({type(exc).__name__}); showing matching context.]\n\n{context[:1500]}"


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
