import logging
import math
import re

from fastapi import APIRouter, Depends, HTTPException
from sqlalchemy import select
from sqlalchemy.orm import Session, joinedload

from .. import schemas
from ..config import settings
from ..db import get_db
from ..models import Chunk, Entity, IngestedItem, MergeAction

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
    def duplicate_proposals(db: Session = Depends(get_db)) -> list[schemas.MergeProposal]:
        entities = (
            db.execute(select(Entity).where(Entity.status != "stale").options(joinedload(Entity.chunks)))
            .unique()
            .scalars()
            .all()
        )
        existing = set(
            (m.entity_a_id, m.entity_b_id)
            for m in db.execute(select(MergeAction)).scalars()
        )
        proposals: list[schemas.MergeProposal] = []
        for i, a in enumerate(entities):
            for b in entities[i + 1 :]:
                if (a.id, b.id) in existing or (b.id, a.id) in existing:
                    continue
                if _similar(a, b) > settings.duplicate_threshold:
                    action = MergeAction(entity_a_id=a.id, entity_b_id=b.id, status="proposed")
                    db.add(action)
                    db.flush()
                    existing.add((a.id, b.id))
                    proposals.append(
                        schemas.MergeProposal(
                            id=action.id, entity_a_id=a.id, entity_b_id=b.id
                        )
                    )
                if len(proposals) >= 25:
                    break
            if len(proposals) >= 25:
                break
        db.commit()
        return proposals

    @router.post("/merge")
    def decide_merge(payload: schemas.MergeDecision, db: Session = Depends(get_db)) -> dict:
        action = db.get(MergeAction, payload.proposal_id)
        if action is None:
            raise HTTPException(status_code=404, detail="Proposal not found")

        if payload.decision == "dismiss":
            action.status = "dismissed"
            db.commit()
            return {"merged": False}

        b = db.get(Entity, payload.entity_b_id)
        a = db.get(Entity, payload.entity_a_id)
        if a is None or b is None:
            raise HTTPException(status_code=404, detail="Entity not found")

        # merge b into a: repoint chunks, keep higher confidence summary
        db.execute(Chunk.__table__.update().where(Chunk.entity_id == b.id).values(entity_id=a.id))
        if b.confidence > a.confidence:
            a.summary, a.reasoning, a.confidence = b.summary, b.reasoning, b.confidence
        if not a.author and b.author:
            a.author = b.author
        a.status = "unverified"
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
