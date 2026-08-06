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
        self._mark_orphans()

    def _mark_orphans(self) -> None:
        """Jobs left 'running' by a previous process (crash/restart) have no
        task anymore — mark them cancelled so they don't stick forever."""
        try:
            with SessionLocal() as db:
                stale = db.execute(select(Job).where(Job.status == "running")).scalars().all()
                for job in stale:
                    job.status = "cancelled"
                    job.finished_at = datetime.now(timezone.utc)
                if stale:
                    db.commit()
                    logger.warning("marked %d orphaned job(s) as cancelled", len(stale))
        except Exception as exc:  # noqa: BLE001
            # runs at import time, before migrations create the table
            logger.debug("orphan sweep skipped (table not ready yet): %s", exc)

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

    def running(self, db: Session) -> list[Job]:
        return list(
            db.execute(
                select(Job).where(Job.status == "running").order_by(Job.created_at.asc())
            ).scalars().all()
        )

    def cancel(self, job_id: int, db: Session | None = None) -> bool:
        """Request cancellation of a running job. The pipeline awaits inside
        the task, so cancelling the asyncio task stops LLM calls mid-flight;
        uncommitted DB writes roll back (pipeline commits once at the end).
        Orphaned jobs (running in DB but no live task, e.g. after restart)
        are marked cancelled directly."""
        task = self._tasks.get(job_id)
        if task is None or task.done():
            with db or SessionLocal() as session:
                job = session.get(Job, job_id)
                if job is not None and job.status == "running":
                    job.status = "cancelled"
                    job.finished_at = datetime.now(timezone.utc)
                    session.commit()
                    logger.warning("job %d cancelled as orphan (no live task)", job_id)
                    return True
            return False
        task.cancel()
        return True

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
        except asyncio.CancelledError:
            logger.info("job %d (%s) cancelled", job_id, kind)
            with SessionLocal() as db:
                job = db.get(Job, job_id)
                if job is None:
                    return
                job.status = "cancelled"
                job.finished_at = datetime.now(timezone.utc)
                db.commit()
            raise
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
