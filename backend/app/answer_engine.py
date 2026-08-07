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
from .models import Chunk, Entity, IngestedItem, Relationship
from .schemas import AskResponse, Citation
from .system_events import record as record_event

logger = logging.getLogger(__name__)

# Graph-based retrieval (B32): relationship kinds ranked by how strongly they
# connect two entities — supersedes/depends_on are the highest-value links for
# answer context; plain "related" is the weakest.
GRAPH_KIND_WEIGHTS = {
    "supersedes": 1.0,
    "depends_on": 0.9,
    "owns": 0.8,
    "blocks": 0.7,
    "related": 0.4,
}
GRAPH_DEFAULT_WEIGHT = 0.5

# B17 planner: tool names the executor can run, and the keyword-signal rules
# that pick extra tools beyond the always-on hybrid vector+keyword search.
TOOL_HYBRID = "hybrid"
TOOL_WHO_KNOWS = "who_knows"
WHO_KNOWS_WEIGHT = 0.8  # RRF weight for the who_knows tool's ranked list

_WHO_KNOWS_RE = re.compile(r"\b(who|whom|owns?|owner|responsible|expert|knows?)\b", re.IGNORECASE)

SYSTEM_PROMPT = """You are AnchorCore, the memory of an organization.
Answer the user's question using ONLY the provided context sections.
Each section is tagged with a section ID like [S1], [S2]...
Always cite the sections you use by ID at the end of the relevant sentence.

Synthesis rules:
- If the context describes a process, workflow, or rules, SYNTHESIZE the
  steps/mechanics explicitly from the sections — don't just say what the
  thing is or that it isn't described.
- Combine evidence across sections (definition + rules + workflow) into one
  coherent explanation.
- Only if the context genuinely has nothing relevant, say so clearly instead
  of guessing.
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
    return _rrf_fuse_multi([vector_hits, keyword_hits], [1.0, keyword_weight])


def _rrf_fuse_multi(lists: list[list[dict]], weights: list[float]) -> list[dict]:
    """RRF over an arbitrary number of ranked lists (B17: planner/executor).

    Each list contributes `weight / (k + rank)`; hits found by multiple tools
    accumulate consensus votes exactly like the two-list case."""
    fused: dict[int, dict] = {}

    def _key(hit: dict) -> int:
        if hit["chunk"] is not None:
            return hit["chunk"].id
        return -hit["entity"].id  # graph hits may carry no chunk

    for hits, weight in zip(lists, weights):
        for rank, hit in enumerate(hits, start=1):
            entry = fused.setdefault(_key(hit), {**hit, "score": 0.0})
            entry["score"] += weight / (RRF_K + rank)
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


def _content_signature(content: str) -> str:
    """Normalized fingerprint for near-duplicate detection: lowercase,
    whitespace collapsed, leading page-number/space noise stripped."""
    text = re.sub(r"\s+", " ", content).strip().lower()
    text = re.sub(r"^\d{1,4}\s+", "", text)  # leading page number
    return text[:160]


def _turn_text(turn) -> str:
    """Render a history turn (dict or pydantic AskTurn) as 'role: content'."""
    if isinstance(turn, dict):
        return f"{turn.get('role', 'user')}: {turn.get('content', '')}"
    role = getattr(turn, "role", "user")
    content = getattr(turn, "content", "")
    return f"{role}: {content}"


class AnswerEngine:
    def __init__(self, embedder: Embedder, settings: SettingsService):
        self.embedder = embedder
        self.settings = settings

    @staticmethod
    def _status_ok(entity) -> bool:
        """Retrieval filter applied uniformly across all tools: stale entities
        are always excluded; disputed entities (B3) are excluded by default
        (ANCHOR_QA_EXCLUDE_DISPUTED). Document chunks carry no entity → allowed."""
        if entity is None:
            return True
        if entity.status == "stale":
            return False
        if entity.status == "disputed" and settings.qa_exclude_disputed:
            return False
        return True

    async def ask(
        self, db: Session, question: str, history: list | None = None, project_id: int | None = None
    ) -> AskResponse:
        """Answer a question; `history` (previous user/assistant turns) enables
        follow-ups: the question is rewritten into a standalone query before
        retrieval, and the conversation is passed to generation. `project_id`
        (B15) scopes retrieval to a project's sources."""
        source_ids = self._project_source_ids(db, project_id)
        history = [t for t in (history or []) if t.content and t.content.strip()]
        query = question
        if history:
            rewritten = await self._rewrite_followup(question, history)
            if rewritten:
                logger.info("follow-up rewritten: %r -> %r", question, rewritten)
                query = rewritten

        # B17: planner picks the tools, executor runs them, fusion synthesizes
        # the evidence bundle (hybrid vector+keyword, who_knows, graph).
        tools = self._plan_tools(query)
        logger.info("planner tools for %r: %s", query, tools)
        evidence = await self._execute_tools(db, query, tools, source_ids=source_ids)

        hits = evidence.get(TOOL_HYBRID) or []
        # B32: graph-based retrieval — connected entities join the evidence
        graph_hits = self._graph_expand(db, hits, source_ids=source_ids)

        all_hits = self._fuse_evidence(evidence, graph_hits)
        if not all_hits:
            return AskResponse(
                answer="No relevant knowledge found yet. Ingest sources first.",
                citations=[],
            )

        sections = []
        for idx, hit in enumerate(all_hits, start=1):
            if hit["chunk"] is not None:
                content = hit["chunk"].content[:2000]
                extra = hit.get("expanded", [])
                if extra:
                    content = content + "\n\n[continued]\n\n" + "\n\n".join(e[:800] for e in extra[:3])
            else:
                content = (hit["entity"].summary if hit.get("entity") else "")[:2000]
            sections.append(f"[S{idx}] {content}")
        context = "\n\n".join(sections)

        answer_text = await self._generate(query, context, history=history)

        citations = []
        for hit in all_hits[:5]:
            chunk, entity, item = hit["chunk"], hit["entity"], hit["item"]
            if entity is not None:
                kind, summary = entity.kind, entity.summary[:200]
            else:
                kind = "document"
                summary = (item.title if item is not None else chunk.source_ref)[:200]
            source_ref = chunk.source_ref if chunk is not None else entity.source_ref
            snippet = chunk.content[:300] if chunk is not None else entity.summary[:300]
            citations.append(
                Citation(
                    entity_id=entity.id if entity is not None else None,
                    kind=kind,
                    summary=summary,
                    source_ref=source_ref,
                    score=round(hit["score"], 3),
                    snippet=snippet,
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

        # diversity cap: one file must not monopolize the results when the
        # corpus has multiple files (Cerebras: cap per file). For a
        # single-file corpus there's nothing to diversify against — allow
        # the full top-k so a big document's sections can all surface.
        cap = int(self.settings.get_float("retrieval_max_per_source", 3))
        if cap > 0:
            item_ids = {hit["chunk"].item_id or (hit["entity"].item_id if hit.get("entity") else None) for hit in fused}
            item_ids.discard(None)
            distinct_items = len(item_ids) or 1
            effective_cap = cap if distinct_items >= 3 else settings.top_k
            per_item: dict[int, int] = {}
            capped: list[dict] = []
            for hit in fused:
                item_id = hit["chunk"].item_id
                if item_id is None and hit.get("entity") is not None:
                    item_id = hit["entity"].item_id
                iid = item_id if item_id is not None else -hit["chunk"].id
                if per_item.get(iid, 0) >= effective_cap:
                    continue
                per_item[iid] = per_item.get(iid, 0) + 1
                capped.append(hit)
            fused = capped

        # dedupe near-identical chunk content from the same item (e.g. TOC
        # entries duplicating body sections) — keep the best-scoring copy
        fused = self._dedupe_similar(fused)

        hits = fused[: settings.top_k]
        return self._expand_context(db, hits)

    @staticmethod
    def _dedupe_similar(hits: list[dict]) -> list[dict]:
        seen: set[tuple[int | None, str]] = set()
        out: list[dict] = []
        for hit in hits:
            chunk = hit["chunk"]
            item_id = chunk.item_id
            if item_id is None and hit.get("entity") is not None:
                item_id = hit["entity"].item_id
            key = (item_id, _content_signature(chunk.content))
            if key in seen:
                continue
            seen.add(key)
            out.append(hit)
        return out

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

    def _graph_expand(self, db: Session, hits: list[dict], source_ids: set[int] | None = None) -> list[dict]:
        """B32: after RRF fusion, walk the entity graph from the winning entities
        1-2 hops and pull connected entities' summaries into context + citations.

        Strong relationship kinds rank higher (supersedes > depends_on > owns >
        blocks > related); further hops decay; fan-out is capped so a hub entity
        can't flood the answer. Pure SQL over `relationships` — no model calls.
        Returns extra hit dicts (marked ``graph``) for entities NOT already in
        the fused list (stale entities excluded). `source_ids` (B15) restricts
        graph-connected entities to the project's sources."""
        hops = max(1, int(settings.retrieval_graph_hops))
        cap = max(0, int(settings.retrieval_graph_max))
        if cap == 0 or not hits:
            return []
        seed_ids = {h["entity"].id for h in hits if h.get("entity") is not None}
        if not seed_ids:
            return []
        visited: set[int] = set(seed_ids)
        frontier: set[int] = seed_ids
        candidates: dict[int, float] = {}
        for hop in range(hops):
            if not frontier:
                break
            rows = db.execute(
                select(Relationship.from_entity_id, Relationship.to_entity_id, Relationship.kind).where(
                    or_(
                        Relationship.from_entity_id.in_(frontier),
                        Relationship.to_entity_id.in_(frontier),
                    )
                )
            ).all()
            next_frontier: set[int] = set()
            for from_id, to_id, kind in rows:
                for a, b in ((from_id, to_id), (to_id, from_id)):
                    if a not in frontier or b in visited:
                        continue
                    weight = GRAPH_KIND_WEIGHTS.get(kind, GRAPH_DEFAULT_WEIGHT) * (0.5**hop)
                    if weight > candidates.get(b, -1.0):
                        candidates[b] = weight
                    next_frontier.add(b)
            visited.update(next_frontier)
            frontier = next_frontier

        if not candidates:
            return []
        ranked = sorted(candidates.items(), key=lambda kv: kv[1], reverse=True)[:cap]
        entity_ids = [eid for eid, _score in ranked]
        entities_q = select(Entity).where(Entity.id.in_(entity_ids))
        chunks_q = select(Chunk).where(Chunk.entity_id.in_(entity_ids))
        if source_ids is not None:
            # only pull graph-connected entities that live inside the project
            entities_q = entities_q.where(Entity.item_id.in_(
                select(IngestedItem.id).where(IngestedItem.source_id.in_(source_ids))
            ))
            chunks_q = chunks_q.where(Chunk.entity_id.in_(
                select(Entity.id).where(Entity.item_id.in_(
                    select(IngestedItem.id).where(IngestedItem.source_id.in_(source_ids))
                ))
            ))
        entities = {e.id: e for e in db.execute(entities_q).scalars().all()}
        chunks = db.execute(chunks_q).scalars().all()
        chunk_by_entity: dict[int, Chunk] = {}
        for c in chunks:
            if c.entity_id not in chunk_by_entity:
                chunk_by_entity[c.entity_id] = c
        out = []
        for eid, score in ranked:
            ent = entities.get(eid)
            if ent is None or not self._status_ok(ent):
                continue
            chunk = chunk_by_entity.get(eid)
            out.append(
                {
                    "chunk": chunk,
                    "entity": ent,
                    "item": ent.item,
                    "source_id": ent.item.source_id if ent.item is not None else None,
                    "score": score,
                    "graph": True,
                }
            )
        return out

    # --- B17: Planner → Executor → Synthesis ---------------------------------

    def _plan_tools(self, query: str) -> list[str]:
        """Planner: choose which retrieval tools to run for this query.

        The always-on `hybrid` tool (vector + FTS5 keyword) covers the default
        path; query signals add specialist tools. Heuristic rules here are the
        cheap, deterministic planner (works without a model — CI-safe); a
        language-model planner can be layered on later without changing the
        executor contract."""
        tools = [TOOL_HYBRID]
        if _WHO_KNOWS_RE.search(query):
            tools.append(TOOL_WHO_KNOWS)
        return tools

    async def _execute_tools(
        self, db: Session, query: str, tools: list[str], source_ids: set[int] | None = None
    ) -> dict[str, list[dict]]:
        """Executor: run the planned tools and normalize each tool's ranked
        hits into a shared evidence shape (chunk/entity/item/source_id/score).

        Tools that share the same vector embedding are batched into one embed
        call; each tool's hits are a ranked list ready for RRF fusion.
        `source_ids` (B15) restricts all tools to a project's sources."""
        evidence: dict[str, list[dict]] = {}

        vector_hits: list[dict] | None = None
        if TOOL_HYBRID in tools or TOOL_WHO_KNOWS in tools:
            try:
                query_embedding = (await self.embedder.embed([query]))[0]
                if TOOL_HYBRID in tools:
                    vector_hits = self._vector_search(db, query_embedding, source_ids=source_ids)
            except Exception as exc:  # noqa: BLE001
                logger.warning("embedding failed (%s); keyword-only retrieval", exc)
                record_event(
                    "qa",
                    "embedding failed; retrieval degraded to keyword-only",
                    level="warning",
                    detail=f"{type(exc).__name__}: {exc}",
                )

        if TOOL_HYBRID in tools:
            keyword_hits = self._keyword_search(db, query, source_ids=source_ids)
            if not keyword_hits and vector_hits is None:
                # embedding AND FTS both unavailable (e.g. pre-migration DB):
                # keep the oldest fallback so degraded mode still answers
                keyword_hits = self._keyword_fallback(db, source_ids=source_ids)
            evidence[TOOL_HYBRID] = self._fuse_and_rank(db, vector_hits, keyword_hits)

        if TOOL_WHO_KNOWS in tools:
            evidence[TOOL_WHO_KNOWS] = self._who_knows_search(db, query, source_ids=source_ids)

        return evidence

    def _who_knows_search(self, db: Session, query: str, source_ids: set[int] | None = None) -> list[dict]:
        """`who_knows` tool: surface entities whose owner/author or summary
        matches the query, ranked by confidence × recency × term overlap.

        Answers "who knows/owns X?" — person + ownership evidence that a plain
        keyword search doesn't weight. Stale entities excluded; results are a
        ranked hit list in the shared evidence shape (empty when no models/DB)."""
        terms = [t for t in _FTS_WORD_RE.findall(query.lower()) if t not in _FTS_STOPWORDS and len(t) >= 3]
        # candidates: entities with an owner/author, or whose summary shares a
        # query term — the "who knows/owns X" surface
        filters = [or_(Entity.author != "", Entity.owner != "")]
        for t in terms:
            filters.append(Entity.summary.contains(t))
        rows = db.execute(self._chunk_query(source_ids).where(or_(*filters))).all()
        hits: list[dict] = []
        halflife = self.settings.get_float("retrieval_age_halflife_days", 365.0)
        for chunk, entity, item in rows:
            if not self._status_ok(entity):
                continue
            blob = " ".join([entity.owner, entity.author, entity.summary]).lower()
            overlap = sum(1 for t in terms if t in blob)
            if overlap == 0:
                continue
            score = entity.confidence * (1.0 + 0.5 * min(overlap, 4)) * _age_decay(entity.created_at, halflife)
            hits.append(
                {
                    "chunk": chunk,
                    "entity": entity,
                    "item": item,
                    "source_id": item.source_id if item is not None else None,
                    "score": score,
                }
            )
        hits.sort(key=lambda r: r["score"], reverse=True)
        return hits[: settings.top_k * 2]

    def _fuse_evidence(
        self, evidence: dict[str, list[dict]], graph_hits: list[dict]
    ) -> list[dict]:
        """Synthesis input: RRF-fuse the planned tools' ranked lists (the hybrid
        list is already internally fused), append graph-connected entities, then
        dedupe. Stale entities were already filtered by each tool."""
        lists: list[list[dict]] = []
        weights: list[float] = []
        if evidence.get(TOOL_HYBRID):
            lists.append(evidence[TOOL_HYBRID])
            weights.append(1.0)
        if evidence.get(TOOL_WHO_KNOWS):
            lists.append(evidence[TOOL_WHO_KNOWS])
            weights.append(WHO_KNOWS_WEIGHT)
        if graph_hits:
            lists.append(graph_hits)
            weights.append(GRAPH_DEFAULT_WEIGHT)

        if not lists:
            return []
        if len(lists) == 1:
            # the hybrid tool's list is already fused + age-decayed + expanded
            fused = sorted(lists[0], key=lambda r: r["score"], reverse=True)
        else:
            fused = _rrf_fuse_multi(lists, weights)
            fused.sort(key=lambda r: r["score"], reverse=True)
        return fused[: settings.top_k]

    def _vector_search(self, db: Session, query_embedding: list[float], source_ids: set[int] | None = None) -> list[dict]:
        rows = db.execute(self._chunk_query(source_ids).where(Chunk.embedding.is_not(None))).all()

        scored = []
        for chunk, entity, item in rows:
            if not self._status_ok(entity):
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

    def _keyword_search(self, db: Session, question: str, source_ids: set[int] | None = None) -> list[dict]:
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
            self._chunk_query(source_ids).where(Chunk.id.in_(list(by_id)))
        ).all()
        hits = []
        for chunk, entity, item in chunks:
            if not self._status_ok(entity):
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

    def _keyword_fallback(self, db: Session, source_ids: set[int] | None = None) -> list[dict]:
        rows = db.execute(
            self._chunk_query(source_ids).order_by(Chunk.created_at.desc()).limit(settings.top_k)
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
            if self._status_ok(entity)
        ]

    def _project_source_ids(self, db: Session, project_id: int | None) -> set[int] | None:
        """B15: resolve a project to its source ids; None = scope to everything."""
        if project_id is None:
            return None
        from .models import Project, project_sources

        rows = db.execute(
            select(project_sources.c.source_id).where(project_sources.c.project_id == project_id)
        ).all()
        if not rows:
            return set()
        return {r[0] for r in rows}

    def _chunk_query(self, source_ids: set[int] | None = None):
        """Chunks of type: entity-summary (entity set) or full-document (item set).
        `source_ids` (B15) restricts to a project's sources."""
        q = (
            select(Chunk, Entity, IngestedItem)
            .outerjoin(Entity, Chunk.entity_id == Entity.id)
            .outerjoin(
                IngestedItem,
                or_(Chunk.item_id == IngestedItem.id, Entity.item_id == IngestedItem.id),
            )
        )
        if source_ids is not None:
            q = q.where(IngestedItem.source_id.in_(source_ids))
        return q

    async def _rewrite_followup(self, question: str, history: list) -> str | None:
        """Rewrite a follow-up ('show the movements for it') into a standalone
        query using the conversation so retrieval has a resolvable referent.
        Returns None (→ use the raw question) when no model is available."""
        answer_base = self.settings.get("answer_base_url") or ""
        api_key = self.settings.get("answer_api_key") or ""
        model = self.settings.get("answer_model") or "gpt-4o-mini"
        is_local = answer_base.startswith(("http://localhost", "http://127.0.0.1"))
        if not is_local and not api_key:
            return None
        transcript = "\n".join(_turn_text(t) for t in history[-6:])
        messages = [
            {
                "role": "system",
                "content": (
                    "You rewrite a user's follow-up question into a standalone "
                    "question that includes all context from the conversation "
                    "needed to answer it alone. Respond with ONLY the rewritten "
                    "question, no preamble."
                ),
            },
            {
                "role": "user",
                "content": f"Conversation:\n{transcript}\n\nFollow-up: {question}",
            },
        ]
        payload = {"model": model, "messages": messages, "temperature": 0.0, "max_tokens": 120}
        headers = {"Authorization": f"Bearer {api_key}"} if api_key else {}
        try:
            async with RetryClient(timeout=30.0) as client:
                resp = await client.post(
                    f"{answer_base.rstrip('/')}/chat/completions",
                    headers=headers,
                    json=payload,
                )
                resp.raise_for_status()
                rewritten = resp.json()["choices"][0]["message"]["content"].strip()
            return rewritten[:500] if rewritten else None
        except Exception as exc:  # noqa: BLE001
            logger.warning("follow-up rewrite failed (%s); using raw question", exc)
            return None

    async def _generate(self, question: str, context: str, history: list | None = None) -> str:
        answer_base = self.settings.get("answer_base_url") or ""
        api_key = self.settings.get("answer_api_key") or ""
        model = self.settings.get("answer_model") or "gpt-4o-mini"
        timeout = self.settings.get_float("answer_timeout", 90.0)

        is_local = answer_base.startswith(("http://localhost", "http://127.0.0.1"))
        if not is_local and not api_key:
            return f"Answer for: {question}\n\n[No model key configured. Matching context:]\n\n{context[:1500]}"

        headers = {"Authorization": f"Bearer {api_key}"} if api_key else {}
        user_content = f"Question: {question}\n\nContext:\n{context}"
        if history:
            transcript = "\n".join(_turn_text(t) for t in history[-6:])
            user_content = f"Conversation so far:\n{transcript}\n\n{user_content}"
        payload = {
            "model": model,
            "messages": [
                {"role": "system", "content": SYSTEM_PROMPT},
                {"role": "user", "content": user_content},
            ],
            "temperature": 0.2,
        }
        effort = (self.settings.get("answer_reasoning_effort") or "none").lower()
        if effort != "none":
            if is_local:
                # Ollama reasoning models (deepseek-r1, qwen3, llama3.3-thinking…)
                payload["think"] = True
                payload.pop("temperature", None)
            else:
                # OpenAI-compatible reasoning models (o-series, gpt-5, …)
                # reject temperature; they decide sampling internally
                payload["reasoning_effort"] = effort
                payload.pop("temperature", None)
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
