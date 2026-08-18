import logging
import math
import re

from fastapi import APIRouter, Depends, HTTPException
from sqlalchemy import delete, or_, select
from sqlalchemy.orm import Session, joinedload

from .. import schemas
from ..config import settings
from ..db import get_db
from ..models import Chunk, Dispute, Entity, IngestedItem, MergeAction, Relationship

logger = logging.getLogger(__name__)


def make_router() -> APIRouter:
    router = APIRouter(prefix="/entities", tags=["entities"])

    @router.get("", response_model=list[schemas.EntityOut])
    def list_entities(
        kind: str | None = None,
        status: str | None = None,
        limit: int = 100,
        db: Session = Depends(get_db),
    ) -> list[Entity]:
        stmt = select(Entity).order_by(Entity.created_at.desc()).limit(limit)
        if kind:
            stmt = stmt.where(Entity.kind == kind)
        if status:
            stmt = stmt.where(Entity.status == status)
        return list(db.execute(stmt).scalars().all())

    @router.get("/{entity_id}", response_model=schemas.EntityOut)
    def get_entity(entity_id: int, db: Session = Depends(get_db)) -> Entity:
        entity = db.get(Entity, entity_id)
        if entity is None:
            raise HTTPException(status_code=404, detail="Entity not found")
        return entity

    @router.patch("/{entity_id}", response_model=schemas.EntityOut)
    def update_entity(
        entity_id: int, payload: schemas.EntityUpdate, db: Session = Depends(get_db)
    ) -> Entity:
        entity = db.get(Entity, entity_id)
        if entity is None:
            raise HTTPException(status_code=404, detail="Entity not found")
        for field, value in payload.model_dump(exclude_none=True).items():
            setattr(entity, field, value)
        db.commit()
        db.refresh(entity)
        return entity

    @router.get("/{entity_id}/related", response_model=list[schemas.EntityOut])
    def related_entities(entity_id: int, db: Session = Depends(get_db)) -> list[Entity]:
        entity = db.get(Entity, entity_id)
        if entity is None:
            raise HTTPException(status_code=404, detail="Entity not found")
        ids = [r.to_entity_id for r in entity.outgoing] + [r.from_entity_id for r in entity.incoming]
        if not ids:
            return []
        return list(db.execute(select(Entity).where(Entity.id.in_(ids))).scalars().all())

    @router.post("/{entity_id}/dispute", response_model=schemas.EntityOut)
    def dispute_entity(
        entity_id: int, payload: schemas.DisputeCreate, db: Session = Depends(get_db)
    ) -> Entity:
        """B3: record a dispute (who/when/why), increment the counter, and
        mark the entity disputed so answers stop citing it."""
        entity = db.get(Entity, entity_id)
        if entity is None:
            raise HTTPException(status_code=404, detail="Entity not found")
        db.add(
            Dispute(
                entity_id=entity.id,
                reason=payload.reason.strip(),
                user=payload.user.strip(),
            )
        )
        entity.dispute_count = (entity.dispute_count or 0) + 1
        entity.status = "disputed"
        db.commit()
        db.refresh(entity)
        return entity

    @router.get("/{entity_id}/disputes", response_model=list[schemas.DisputeOut])
    def dispute_history(entity_id: int, db: Session = Depends(get_db)) -> list[Dispute]:
        """B3: the audit trail for an entity, newest first."""
        entity = db.get(Entity, entity_id)
        if entity is None:
            raise HTTPException(status_code=404, detail="Entity not found")
        return list(
            db.execute(
                select(Dispute)
                .where(Dispute.entity_id == entity_id)
                .order_by(Dispute.created_at.desc())
            ).scalars()
        )

    @router.get("/{entity_id}/context")
    def entity_context(entity_id: int, db: Session = Depends(get_db)) -> dict:
        """B27: expanded review context — window plus neighbouring sections so the
        reviewer sees the surrounding passage, not just the classifier fragment."""
        entity = db.get(Entity, entity_id)
        if entity is None:
            raise HTTPException(status_code=404, detail="Entity not found")
        item = db.get(IngestedItem, entity.item_id)
        if item is None:
            return {
                "entity_id": entity.id,
                "source_name": None,
                "source_ref": entity.source_ref,
                "window_text": entity.window_text or "",
                "expanded_before": [],
                "expanded_after": [],
                "full_text": "",
                "highlight": entity.summary,
                "item_title": "",
            }
        from ..models import Source

        source_name = None
        try:
            src_obj = db.get(Source, item.source_id)
            source_name = src_obj.name if src_obj else None
        except Exception:
            source_name = None

        window = entity.window_text or ""
        before: list[str] = []
        after: list[str] = []
        full = item.text or ""
        if full and window:
            try:
                from ..chunking import chunk_document

                chunks = chunk_document(full)
                idx = None
                needle = window[:160].strip()
                for i, c in enumerate(chunks):
                    if needle and needle in c:
                        idx = i
                        break
                if idx is None and entity.window_index:
                    idx = max(0, min(entity.window_index - 1, len(chunks) - 1))
                if idx is not None:
                    if idx > 0:
                        before = [chunks[idx - 1]]
                    if idx + 1 < len(chunks):
                        after = [chunks[idx + 1]]
            except Exception:
                pass

        return {
            "entity_id": entity.id,
            "source_name": source_name,
            "source_ref": entity.source_ref,
            "window_text": window,
            "expanded_before": before,
            "expanded_after": after,
            "full_text": full[:16000],
            "highlight": entity.summary,
            "item_title": item.title,
        }

    return router


def review_router() -> APIRouter:
    router = APIRouter(prefix="/review", tags=["review"])

    @router.get("/low-confidence", response_model=list[schemas.EntityOut])
    def low_confidence(db: Session = Depends(get_db)) -> list[Entity]:
        return list(
            db.execute(
                select(Entity)
                .where(
                    Entity.confidence < settings.low_confidence_threshold,
                    Entity.status == "unverified",
                )
                .order_by(Entity.confidence.asc())
                .limit(100)
            ).scalars()
        )

    @router.get("/duplicates", response_model=list[schemas.MergeProposal])
    def duplicate_proposals(
        limit: int = 25,
        offset: int = 0,
        kind: str | None = None,
        db: Session = Depends(get_db),
    ) -> list[schemas.MergeProposal]:
        """B38: O(n²) duplicate scan, now paginated + bounded. `limit` caps the
        returned proposals, `offset` skips already-returned pages, `kind`
        restricts the candidate pool. On very large corpora (>500 candidates)
        the pool is sampled to the 200 most-recent so the O(n²) scan stays
        responsive instead of stalling the request."""
        limit = max(1, min(limit, 100))
        offset = max(0, offset)
        stmt = select(Entity).where(Entity.status != "stale").options(joinedload(Entity.chunks))
        if kind:
            stmt = stmt.where(Entity.kind == kind)
        entities = db.execute(stmt).unique().scalars().all()
        if len(entities) > 500:
            # early-exit: sample the most recent 200 for the scan
            entities = sorted(entities, key=lambda e: e.created_at, reverse=True)[:200]
        existing = set(
            (m.entity_a_id, m.entity_b_id)
            for m in db.execute(select(MergeAction)).scalars()
        )
        proposals: list[schemas.MergeProposal] = []
        seen: set[int] = set()
        for i, a in enumerate(entities):
            if a.id in seen:
                continue
            for b in entities[i + 1 :]:
                if b.id in seen:
                    continue
                if (a.id, b.id) in existing or (b.id, a.id) in existing:
                    continue
                if _similar(a, b) > settings.duplicate_threshold:
                    action = MergeAction(entity_a_id=a.id, entity_b_id=b.id, status="proposed")
                    db.add(action)
                    db.flush()
                    existing.add((a.id, b.id))
                    proposals.append(schemas.MergeProposal(id=action.id, entity_a_id=a.id, entity_b_id=b.id))
                    seen.add(a.id)
                    seen.add(b.id)
                if len(proposals) >= offset + limit:
                    break
            if len(proposals) >= offset + limit:
                break
        db.commit()
        return proposals[offset:]

    @router.post("/merge")
    def decide_merge(payload: schemas.MergeDecision, db: Session = Depends(get_db)) -> dict:
        action = db.get(MergeAction, payload.proposal_id)
        if action is None:
            raise HTTPException(status_code=404, detail="Proposal not found")

        if payload.decision == "dismiss":
            action.status = "dismissed"
            db.commit()
            return {"merged": False}

        b = db.get(Entity, action.entity_b_id)
        a = db.get(Entity, action.entity_a_id)
        if a is None or b is None:
            raise HTTPException(status_code=404, detail="Entity not found")

        # merge b into a: repoint chunks, keep higher confidence summary
        db.execute(Chunk.__table__.update().where(Chunk.entity_id == b.id).values(entity_id=a.id))
        if b.confidence > a.confidence:
            a.summary, a.reasoning, a.confidence = b.summary, b.reasoning, b.confidence
        if not a.author and b.author:
            a.author = b.author
        a.status = "unverified"
        db.execute(
            delete(Relationship).where(
                or_(Relationship.from_entity_id == b.id, Relationship.to_entity_id == b.id)
            )
        )
        db.execute(
            delete(MergeAction).where(
                or_(MergeAction.entity_a_id == b.id, MergeAction.entity_b_id == b.id)
            )
        )
        db.delete(b)
        action.status = "merged"
        db.commit()
        return {"merged": True, "entity_id": a.id}

    return router


def _similar(a: Entity, b: Entity) -> float:
    ta = re.sub(r"[^a-z0-9 ]", " ", a.summary.lower())
    tb = re.sub(r"[^a-z0-9 ]", " ", b.summary.lower())
    wa, wb = set(ta.split()), set(tb.split())
    if not wa or not wb:
        return 0.0
    return len(wa & wb) / math.sqrt(len(wa) * len(wb))
