from fastapi import APIRouter, Depends, HTTPException
from sqlalchemy import select
from sqlalchemy.orm import Session

from .. import schemas
from ..db import get_db
from ..jobs import JobManager
from ..models import Source
from ..pipeline import IngestionPipeline
from ..secrets import SecretStore, resolve_source_config, store_source_config


def make_router(
    pipeline: IngestionPipeline, secrets: SecretStore, scheduler, jobs: JobManager
) -> APIRouter:
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

    @router.post("/{source_id}/sync", response_model=schemas.JobOut, status_code=202)
    async def sync_source(source_id: int, db: Session = Depends(get_db)) -> schemas.JobOut:
        _require_source(db, source_id)
        return jobs.get(db, jobs.start(source_id, kind="sync"))

    @router.post("/{source_id}/reclassify", response_model=schemas.JobOut, status_code=202)
    async def reclassify_source(source_id: int, db: Session = Depends(get_db)) -> schemas.JobOut:
        """Force re-classification of all items in the source (e.g. after
        installing the LLM model or switching to a better classifier)."""
        _require_source(db, source_id)
        return jobs.get(db, jobs.start(source_id, kind="reclassify"))

    @router.get("/{source_id}/config")
    def get_config(source_id: int, db: Session = Depends(get_db)) -> dict:
        source = db.get(Source, source_id)
        if source is None:
            raise HTTPException(status_code=404, detail="Source not found")
        config = resolve_source_config(source, secrets)
        # never ship credentials back to the UI
        for field in ("token", "api_key", "password"):
            if field in config:
                config[field] = "***set***"
        return config

    @router.get("/jobs", response_model=list[schemas.JobOut])
    def list_jobs(
        source_id: int | None = None, limit: int = 10, db: Session = Depends(get_db)
    ) -> list[schemas.JobOut]:
        return jobs.recent(db, source_id=source_id, limit=min(limit, 100))

    @router.get("/jobs/running", response_model=list[schemas.JobOut])
    def list_running_jobs(db: Session = Depends(get_db)) -> list[schemas.JobOut]:
        return jobs.running(db)

    @router.get("/jobs/{job_id}", response_model=schemas.JobOut)
    def get_job(job_id: int, db: Session = Depends(get_db)) -> schemas.JobOut:
        job = jobs.get(db, job_id)
        if job is None:
            raise HTTPException(status_code=404, detail="Job not found")
        return job

    return router


def _require_source(db: Session, source_id: int) -> Source:
    source = db.get(Source, source_id)
    if source is None:
        raise HTTPException(status_code=404, detail="Source not found")
    return source
