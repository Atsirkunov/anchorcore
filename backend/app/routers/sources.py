import json
from datetime import datetime, timezone

from fastapi import APIRouter, Depends, HTTPException
from sqlalchemy import select
from sqlalchemy.orm import Session

from .. import schemas
from ..connectors import ConnectorError
from ..db import get_db
from ..models import Source
from ..pipeline import IngestionPipeline
from ..secrets import SecretStore, store_source_config


def make_router(pipeline: IngestionPipeline, secrets: SecretStore, scheduler) -> APIRouter:
    router = APIRouter(prefix="/sources", tags=["sources"])

    @router.post("", response_model=schemas.SourceOut, status_code=201)
    def create_source(payload: schemas.SourceCreate, db: Session = Depends(get_db)) -> Source:
        source = Source(connector=payload.connector, name=payload.name)
        db.add(source)
        db.flush()
        store_source_config(db, source, payload.config, secrets)
        db.commit()
        db.refresh(source)
        scheduler.reload_sources()
        return source

    @router.get("", response_model=list[schemas.SourceOut])
    def list_sources(db: Session = Depends(get_db)) -> list[Source]:
        return list(db.execute(select(Source).order_by(Source.created_at.desc())).scalars().all())

    @router.delete("/{source_id}")
    def delete_source(source_id: int, db: Session = Depends(get_db)) -> dict:
        source = db.get(Source, source_id)
        if source is None:
            raise HTTPException(status_code=404, detail="Source not found")
        db.delete(source)
        db.commit()
        scheduler.reload_sources()
        return {"deleted": True}

    @router.post("/{source_id}/sync")
    async def sync_source(source_id: int, db: Session = Depends(get_db)) -> dict:
        source = db.get(Source, source_id)
        if source is None:
            raise HTTPException(status_code=404, detail="Source not found")
        try:
            return await pipeline.sync_source(db, source)
        except ConnectorError as exc:
            raise HTTPException(status_code=400, detail=str(exc)) from exc

    @router.post("/{source_id}/reclassify")
    async def reclassify_source(source_id: int, db: Session = Depends(get_db)) -> dict:
        """Force re-classification of all items in the source (e.g. after
        installing the LLM model or switching to a better classifier)."""
        source = db.get(Source, source_id)
        if source is None:
            raise HTTPException(status_code=404, detail="Source not found")
        try:
            return await pipeline.sync_source(db, source, force_reclassify=True)
        except ConnectorError as exc:
            raise HTTPException(status_code=400, detail=str(exc)) from exc

    @router.get("/{source_id}/config")
    def get_config(source_id: int, db: Session = Depends(get_db)) -> dict:
        source = db.get(Source, source_id)
        if source is None:
            raise HTTPException(status_code=404, detail="Source not found")
        return resolve_source_config(source, secrets)

    return router
