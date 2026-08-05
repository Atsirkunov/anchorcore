import json
import logging
import re

from sqlalchemy import select, text
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

_FTS_WORD_RE = re.compile(r"[a-z0-9_§\-]+", re.IGNORECASE)
_FTS_STOPWORDS = {
    "a", "an", "the", "and", "or", "but", "of", "in", "on", "at", "to", "for",
    "with", "about", "is", "are", "was", "were", "be", "been", "being", "am",
    "do", "does", "did", "have", "has", "had", "will", "would", "can", "could",
    "should", "shall", "may", "might", "must", "what", "which", "who", "whom",
    "whose", "when", "where", "why", "how", "this", "that", "these", "those",
    "it", "its", "not", "no", "so", "if", "then", "than", "too", "very", "s",
    "t", "you", "your", "we", "our", "they", "their", "i", "me", "my",
}


def _fts_match_query(question: str) -> str | None:
    """Turn a question into an FTS5 MATCH expression: OR of quoted terms."""
    terms = [
        t
        for t in _FTS_WORD_RE.findall(question.lower())
        if len(t) >= 2 and t not in _FTS_STOPWORDS
    ]
    if not terms:
        return None
    # quote each term; FTS5 quotes can't be escaped, and our token regex
    # never produces quotes, so plain wrapping is safe
    return " OR ".join(f'"{t}"' for t in terms)


class AnswerEngine:
    def __init__(self, embedder: Embedder):
        self.embedder = embedder

    async def ask(self, db: Session, question: str) -> AskResponse:
        vector_hits: list[dict] | None = None
        try:
            query_embedding = (await self.embedder.embed([question]))[0]
            vector_hits = self._vector_search(db, query_embedding)
        except Exception as exc:  # noqa: BLE001
            logger.warning("embedding/search failed (%s); keyword-only retrieval", exc)
            record_event(
                "qa",
                "embedding failed; retrieval degraded to keyword-only",
                level="warning",
                detail=f"{type(exc).__name__}: {exc}",
            )

        keyword_hits = self._keyword_search(db, question)
        if not keyword_hits and vector_hits is None:
            # embedding AND FTS both unavailable (e.g. pre-migration DB):
            # keep the old newest-chunks fallback so degraded mode still answers
            keyword_hits = self._keyword_fallback(db)
        hits = self._merge_hits(vector_hits, keyword_hits)

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

    def _keyword_search(self, db: Session, question: str) -> list[dict]:
        """FTS5 keyword search over chunks (bm25 ranking). Returns [] when
        the FTS table is missing or the query has no useful terms."""
        match = _fts_match_query(question)
        if match is None:
            return []
        try:
            rows = db.execute(
                text(
                    "SELECT rowid, bm25(chunks_fts) AS rank FROM chunks_fts "
                    "WHERE chunks_fts MATCH :q ORDER BY rank LIMIT :limit"
                ),
                {"q": match, "limit": settings.top_k * 4},
            ).all()
        except Exception as exc:  # noqa: BLE001
            logger.warning("FTS keyword search unavailable (%s)", exc)
            return []
        if not rows:
            return []
        by_id = {row[0]: row[1] for row in rows}  # rowid -> bm25 rank (more negative = better)

        chunks = db.execute(
            self._chunk_query().where(Chunk.id.in_(list(by_id)))
        ).all()
        hits = []
        for chunk, entity, item in chunks:
            if entity is not None and entity.status == "stale":
                continue
            # bm25 returns negative scores; more negative = better match
            hits.append(
                {
                    "chunk": chunk,
                    "entity": entity,
                    "item": item,
                    "score": -by_id[chunk.id],
                }
            )
        hits.sort(key=lambda r: r["score"], reverse=True)
        return hits

    def _merge_hits(
        self, vector_hits: list[dict] | None, keyword_hits: list[dict] | None
    ) -> list[dict]:
        """Hybrid merge: normalized bm25 + cosine, configurable keyword weight."""
        if not vector_hits and not keyword_hits:
            return []
        if not vector_hits:
            return keyword_hits or []
        if not keyword_hits:
            return vector_hits

        kw_max = max(h["score"] for h in keyword_hits) or 1.0
        vec_max = max(h["score"] for h in vector_hits) or 1.0
        weight = settings.retrieval_keyword_weight

        combined: dict[int, dict] = {}
        for hit in vector_hits:
            combined[hit["chunk"].id] = {**hit, "vec": hit["score"] / vec_max, "kw": 0.0}
        for hit in keyword_hits:
            entry = combined.setdefault(hit["chunk"].id, {**hit, "vec": 0.0})
            entry["kw"] = hit["score"] / kw_max
            entry["chunk"] = hit["chunk"]
            entry["entity"] = hit["entity"]
            entry["item"] = hit["item"]

        results = []
        for entry in combined.values():
            entry["score"] = weight * entry["kw"] + (1 - weight) * entry["vec"]
            results.append(entry)
        results.sort(key=lambda r: r["score"], reverse=True)
        return results[: settings.top_k]

    def _keyword_fallback(self, db: Session) -> list[dict]:
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
