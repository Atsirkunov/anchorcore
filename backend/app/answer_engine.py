import json
import logging
import re
from datetime import datetime, timezone

from sqlalchemy import or_, select, text
from sqlalchemy.orm import Session

from .app_settings import SettingsService
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

RRF_K = 60.0

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


def _rrf_fuse(
    vector_hits: list[dict], keyword_hits: list[dict], keyword_weight: float
) -> list[dict]:
    """Reciprocal rank fusion: score = Σ w / (k + rank) across ranked lists.

    Consensus across retrievers beats a single strong vote; needs no score
    normalization (Cerebras KB design, k=60).
    """
    fused: dict[int, dict] = {}
    for rank, hit in enumerate(vector_hits, start=1):
        entry = fused.setdefault(hit["chunk"].id, {**hit, "score": 0.0})
        entry["score"] += 1.0 / (RRF_K + rank)
    for rank, hit in enumerate(keyword_hits, start=1):
        entry = fused.setdefault(hit["chunk"].id, {**hit, "score": 0.0})
        entry["score"] += keyword_weight / (RRF_K + rank)
    return list(fused.values())


def _age_decay(created_at, halflife_days: float) -> float:
    """0.5^(age/halflife): recent wins when relevance is otherwise equal."""
    if created_at is None:
        return 1.0
    try:
        if created_at.tzinfo is None:
            created_at = created_at.replace(tzinfo=timezone.utc)
        age_days = max(0.0, (datetime.now(timezone.utc) - created_at).total_seconds() / 86400)
    except (TypeError, OverflowError):
        return 1.0
    if halflife_days <= 0:
        return 1.0
    return 0.5 ** (age_days / halflife_days)


class AnswerEngine:
    def __init__(self, embedder: Embedder, settings: SettingsService):
        self.embedder = embedder
        self.settings = settings

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
        hits = self._fuse_and_rank(db, vector_hits, keyword_hits)

        if not hits:
            return AskResponse(
                answer="No relevant knowledge found yet. Ingest sources first.",
                citations=[],
            )

        sections = []
        for idx, hit in enumerate(hits, start=1):
            content = hit["chunk"].content[:2000]
            extra = hit.get("expanded", [])
            if extra:
                content = content + "\n\n[continued]\n\n" + "\n\n".join(e[:800] for e in extra[:3])
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

    def _fuse_and_rank(
        self, db: Session, vector_hits: list[dict] | None, keyword_hits: list[dict] | None
    ) -> list[dict]:
        """RRF fusion → age decay → per-source diversity cap → top_k → context expansion."""
        if not vector_hits and not keyword_hits:
            return []
        if not vector_hits:
            fused = [{**h, "score": 1.0 / (RRF_K + rank)} for rank, h in enumerate(keyword_hits, start=1)]
        elif not keyword_hits:
            fused = [{**h, "score": 1.0 / (RRF_K + rank)} for rank, h in enumerate(vector_hits, start=1)]
        else:
            fused = _rrf_fuse(vector_hits, keyword_hits, self.settings.get_float("retrieval_keyword_weight", 1.0))

        for hit in fused:
            hit["score"] *= _age_decay(
                hit["chunk"].created_at, self.settings.get_float("retrieval_age_halflife_days", 365.0)
            )

        fused.sort(key=lambda r: r["score"], reverse=True)

        # diversity cap: one source must not monopolize the results
        cap = int(self.settings.get_float("retrieval_max_per_source", 3))
        if cap > 0:
            per_source: dict[int, int] = {}
            capped: list[dict] = []
            for hit in fused:
                sid = hit.get("source_id") or 0
                if per_source.get(sid, 0) >= cap:
                    continue
                per_source[sid] = per_source.get(sid, 0) + 1
                capped.append(hit)
            fused = capped

        hits = fused[: settings.top_k]
        return self._expand_context(db, hits)

    def _expand_context(self, db: Session, hits: list[dict]) -> list[dict]:
        """Pull neighboring chunks of the same item so section boundaries
        (heading, preconditions, caveats) aren't lost (Cerebras learning)."""
        window = settings.retrieval_context_window
        if window <= 0 or not hits:
            return hits
        hit_ids = {h["chunk"].id for h in hits}
        for hit in hits:
            chunk = hit["chunk"]
            item_id = chunk.item_id
            if item_id is None and hit.get("entity") is not None:
                item_id = hit["entity"].item_id
            if item_id is None:
                continue
            siblings = db.execute(
                select(Chunk.id, Chunk.content)
                .where(Chunk.item_id == item_id)
                .order_by(Chunk.id)
            ).all()
            idx = next((i for i, (cid, _c) in enumerate(siblings) if cid == chunk.id), None)
            if idx is None:
                continue
            neighbors = [
                content
                for cid, content in siblings[max(0, idx - window) : idx + window + 1]
                if cid not in hit_ids and cid != chunk.id
            ]
            if neighbors:
                hit["expanded"] = neighbors
        return hits

    def _vector_search(self, db: Session, query_embedding: list[float]) -> list[dict]:
        rows = db.execute(self._chunk_query().where(Chunk.embedding.is_not(None))).all()

        scored = []
        for chunk, entity, item in rows:
            if entity is not None and entity.status == "stale":
                continue
            vector = unpack_f32(chunk.embedding, len(query_embedding))
            score = _cosine(query_embedding, vector)
            scored.append(
                {
                    "chunk": chunk,
                    "entity": entity,
                    "item": item,
                    "source_id": item.source_id if item is not None else None,
                    "score": score,
                }
            )

        scored.sort(key=lambda r: r["score"], reverse=True)
        return [hit for hit in scored[: settings.top_k * 4] if hit["score"] > 0.2]

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
                    "source_id": item.source_id if item is not None else None,
                    "score": -by_id[chunk.id],
                }
            )
        hits.sort(key=lambda r: r["score"], reverse=True)
        return hits

    def _keyword_fallback(self, db: Session) -> list[dict]:
        rows = db.execute(
            self._chunk_query().order_by(Chunk.created_at.desc()).limit(settings.top_k)
        ).all()
        return [
            {
                "chunk": chunk,
                "entity": entity,
                "item": item,
                "source_id": item.source_id if item is not None else None,
                "score": 0.0,
            }
            for chunk, entity, item in rows
            if entity is None or entity.status != "stale"
        ]

    def _chunk_query(self):
        """Chunks of type: entity-summary (entity set) or full-document (item set)."""
        return (
            select(Chunk, Entity, IngestedItem)
            .outerjoin(Entity, Chunk.entity_id == Entity.id)
            .outerjoin(
                IngestedItem,
                or_(Chunk.item_id == IngestedItem.id, Entity.item_id == IngestedItem.id),
            )
        )

    async def _generate(self, question: str, context: str) -> str:
        answer_base = self.settings.get("answer_base_url") or ""
        api_key = self.settings.get("answer_api_key") or ""
        model = self.settings.get("answer_model") or "gpt-4o-mini"
        timeout = self.settings.get_float("answer_timeout", 90.0)

        is_local = answer_base.startswith(("http://localhost", "http://127.0.0.1"))
        if not is_local and not api_key:
            return f"Answer for: {question}\n\n[No model key configured. Matching context:]\n\n{context[:1500]}"

        headers = {"Authorization": f"Bearer {api_key}"} if api_key else {}
        payload = {
            "model": model,
            "messages": [
                {"role": "system", "content": SYSTEM_PROMPT},
                {"role": "user", "content": f"Question: {question}\n\nContext:\n{context}"},
            ],
            "temperature": 0.2,
        }
        try:
            async with RetryClient(timeout=timeout) as client:
                resp = await client.post(
                    f"{answer_base.rstrip('/')}/chat/completions",
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
