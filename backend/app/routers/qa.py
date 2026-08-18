from fastapi import APIRouter, Depends
from sqlalchemy import select
from sqlalchemy.orm import Session

from .. import schemas
from ..answer_engine import AnswerEngine
from ..db import get_db
from ..models import Entity


def make_router(answer_engine: AnswerEngine) -> APIRouter:
    router = APIRouter(prefix="/qa", tags=["qa"])

    @router.post("", response_model=schemas.AskResponse)
    async def ask(payload: schemas.AskRequest, db: Session = Depends(get_db)) -> schemas.AskResponse:
        return await answer_engine.ask(
            db,
            payload.question,
            history=payload.history,
            project_id=payload.project_id,
            public_only=payload.public_only,
        )

    @router.post("/public", response_model=schemas.AskResponse)
    async def ask_public(payload: schemas.AskRequest, db: Session = Depends(get_db)) -> schemas.AskResponse:
        """B30: shared-link / agent-friendly ask — answers ONLY from `public`
        sources. Non-public content is never retrieved, so this endpoint is safe
        to expose to share links, MCP tools, and agents."""
        return await answer_engine.ask(
            db,
            payload.question,
            history=payload.history,
            project_id=payload.project_id,
            public_only=True,
        )

    @router.post("/search", response_model=schemas.SearchResponse)
    async def search(payload: schemas.SearchRequest, db: Session = Depends(get_db)) -> schemas.SearchResponse:
        """B14: raw retrieval for agents/harnesses — ranked chunks/entities with
        provenance and scores, no LLM generation."""
        hits = await answer_engine.search(db, payload.query, k=payload.k, project_id=payload.project_id)
        return schemas.SearchResponse(hits=[schemas.SearchHit(**h) for h in hits])

    @router.get("/context", response_model=list[schemas.EntityOut])
    def context_snapshot(db: Session = Depends(get_db)) -> list[Entity]:
        """What the memory currently contains — useful for the UI state view."""
        return list(
            db.execute(
                select(Entity).where(Entity.status != "stale").order_by(Entity.created_at.desc()).limit(50)
            ).scalars()
        )

    return router
