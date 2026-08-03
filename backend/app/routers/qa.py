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
        return await answer_engine.ask(db, payload.question)

    @router.get("/context", response_model=list[schemas.EntityOut])
    def context_snapshot(db: Session = Depends(get_db)) -> list[Entity]:
        """What the memory currently contains — useful for the UI state view."""
        return list(
            db.execute(
                select(Entity).where(Entity.status != "stale").order_by(Entity.created_at.desc()).limit(50)
            ).scalars()
        )

    return router
