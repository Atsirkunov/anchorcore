import logging
import re
from datetime import datetime, timezone
from pathlib import Path

from fastapi import APIRouter, Depends, HTTPException
from fastapi.responses import FileResponse
from sqlalchemy import func, select
from sqlalchemy.orm import Session

from .. import schemas
from ..app_settings import SettingsService
from ..config import settings
from ..db import get_db
from ..models import Chunk, Source, SystemEvent
from ..status import (
    answer_provider,
    classifier_is_local,
    embedder_is_local,
    missing_ollama_models,
    ollama_reachable,
)
from ..throughput import throughput

logger = logging.getLogger(__name__)

APP_VERSION = "1.0.2"

_LOG_NAME_RE = re.compile(r"^anchorcore\.log(\.\d+)?$")


def make_router(scheduler, settings_svc: SettingsService) -> APIRouter:
    router = APIRouter(prefix="/system", tags=["system"])

    @router.get("/status")
    async def status(db: Session = Depends(get_db)) -> dict:
        pending = db.execute(
            select(func.count()).select_from(Chunk).where(Chunk.embedding.is_(None))
        ).scalar_one()
        failing_sources = list(
            db.execute(
                select(Source).where(Source.error_count > 0, Source.enabled.is_(True))
            ).scalars()
        )
        missing = missing_ollama_models(settings_svc)
        return {
            "version": APP_VERSION,
            "data_dir": str(settings.data_dir),
            "database": settings.resolved_database_url,
            "ollama": {
                "reachable": ollama_reachable(settings_svc),
                "base_url": settings_svc.get("ollama_base_url") or "",
                "classifier_model": settings_svc.get("classifier_model") or "",
                "embed_model": settings_svc.get("embed_model") or "",
                "missing_models": missing,
            },
            "classifier": {
                "provider": "local" if classifier_is_local(settings_svc) else "cloud",
                "base_url": (
                    settings_svc.get("classifier_base_url")
                    or settings_svc.get("ollama_base_url")
                    or ""
                ),
                "model": settings_svc.get("classifier_model") or "",
                **throughput.snapshot(),
                "concurrency": settings.classifier_concurrency,
            },
            "embedder": {
                "provider": "local" if embedder_is_local(settings_svc) else "cloud",
                "base_url": (
                    settings_svc.get("embed_base_url")
                    or settings_svc.get("ollama_base_url")
                    or ""
                ),
                "model": settings_svc.get("embed_model") or "",
            },
            "answer": {
                "provider": answer_provider(settings_svc),
                "model": settings_svc.get("answer_model") or "",
                "base_url": settings_svc.get("answer_base_url") or "",
            },
            "tasks": scheduler.task_states(),
            "pending_embeddings": pending,
            "failing_sources": [
                {"id": s.id, "name": s.name, "error": s.last_error, "count": s.error_count}
                for s in failing_sources
            ],
        }

    @router.get("/errors", response_model=list[schemas.SystemEventOut])
    def list_errors(
        limit: int = 50,
        component: str | None = None,
        level: str | None = None,
        db: Session = Depends(get_db),
    ) -> list[schemas.SystemEventOut]:
        query = select(SystemEvent)
        if component:
            query = query.where(SystemEvent.component == component)
        if level:
            query = query.where(SystemEvent.level == level)
        events = list(
            db.execute(query.order_by(SystemEvent.created_at.desc()).limit(min(limit, 200))).scalars()
        )
        source_ids = [e.source_id for e in events if e.source_id is not None]
        source_names = {
            s.id: s.name
            for s in db.execute(select(Source).where(Source.id.in_(source_ids))).scalars()
        } if source_ids else {}
        return [
            schemas.SystemEventOut(
                id=e.id,
                component=e.component,
                level=e.level,
                source_id=e.source_id,
                source_name=source_names.get(e.source_id),
                message=e.message,
                detail=e.detail,
                created_at=e.created_at,
            )
            for e in events
        ]

    @router.get("/logs", response_model=list[schemas.LogFileOut])
    def list_logs() -> list[dict]:
        files = []
        log_dir = Path(settings.data_dir)
        if log_dir.exists():
            for path in sorted(log_dir.glob("anchorcore.log*")):
                if not _LOG_NAME_RE.match(path.name):
                    continue
                stat = path.stat()
                files.append(
                    {
                        "name": path.name,
                        "size": stat.st_size,
                        "modified": datetime.fromtimestamp(stat.st_mtime, tz=timezone.utc),
                    }
                )
        return files

    @router.get("/logs/{filename}")
    def download_log(filename: str) -> FileResponse:
        if not _LOG_NAME_RE.fullmatch(filename):
            raise HTTPException(status_code=404, detail="Log file not found")
        path = Path(settings.data_dir) / filename
        if not path.is_file():
            raise HTTPException(status_code=404, detail="Log file not found")
        return FileResponse(path, media_type="text/plain", filename=filename)

    return router
