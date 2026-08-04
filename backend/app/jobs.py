import asyncio
import json
import logging
from datetime import datetime, timezone

from sqlalchemy import select
from sqlalchemy.orm import Session

from .db import SessionLocal
from .models import Job

logger = logging.getLogger(__name__)


class JobManager:
    """Runs sync/reclassify as background tasks so long operations never block
    an HTTP request or look frozen. Jobs carry progress (processed/total) and
    persist a result + error for the job-history surface."""

    def __init__(self, pipeline):
        self.pipeline = pipeline
        self._tasks: dict[int, asyncio.Task] = {}

    def start(self, source_id: int, kind: str) -> int:
        """Create a job row and start it in the background. Returns the job id."""
        with SessionLocal() as db:
            job = Job(source_id=source_id, kind=kind)
            db.add(job)
            db.commit()
            db.refresh(job)
            job_id = job.id
        self._tasks[job_id] = asyncio.create_task(self._run(job_id), name=f"job-{job_id}")
        return job_id

    def get(self, db: Session, job_id: int) -> Job | None:
        return db.get(Job, job_id)

    def recent(self, db: Session, source_id: int | None = None, limit: int = 10) -> list[Job]:
        query = select(Job)
        if source_id is not None:
            query = query.where(Job.source_id == source_id)
        return list(
            db.execute(query.order_by(Job.created_at.desc()).limit(limit)).scalars().all()
        )

    async def _run(self, job_id: int) -> None:
        async def progress(processed: int, total: int) -> None:
            with SessionLocal() as db:
                job = db.get(Job, job_id)
                if job is None:
                    return
                job.processed = processed
                job.total = total
                db.commit()

        with SessionLocal() as db:
            job = db.get(Job, job_id)
            if job is None:
                return
            job.status = "running"
            job.started_at = datetime.now(timezone.utc)
            job.error = None
            # read scalars BEFORE commit expires the row / session closes
            source_id = job.source_id
            kind = job.kind
            db.commit()
        try:
            with SessionLocal() as db:
                from .models import Source

                source = db.get(Source, source_id)
                if source is None:
                    raise ValueError(f"source {source_id} no longer exists")
                result = await self.pipeline.sync_source(
                    db, source, force_reclassify=(kind == "reclassify"), progress=progress
                )
            with SessionLocal() as db:
                job = db.get(Job, job_id)
                if job is None:
                    return
                job.status = "done"
                job.result = json.dumps(result)
                job.finished_at = datetime.now(timezone.utc)
                db.commit()
        except Exception as exc:  # noqa: BLE001
            logger.error("job %d (%s) failed: %s", job_id, kind, exc)
            with SessionLocal() as db:
                job = db.get(Job, job_id)
                if job is None:
                    return
                job.status = "failed"
                job.error = f"{type(exc).__name__}: {exc}"[:2000]
                job.finished_at = datetime.now(timezone.utc)
                db.commit()
        finally:
            self._tasks.pop(job_id, None)
