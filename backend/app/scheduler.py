import asyncio
import logging
from datetime import datetime, timezone

from sqlalchemy import select

from .config import settings
from .db import SessionLocal
from .models import Source

logger = logging.getLogger(__name__)


class Scheduler:
    """Background loop: periodic Jira polls + hourly folder re-scans."""

    def __init__(self, pipeline):
        self.pipeline = pipeline
        self._tasks: list[asyncio.Task] = []

    def start(self) -> None:
        self._tasks = [
            asyncio.create_task(self._poll_loop("jira", settings.jira_poll_minutes)),
            asyncio.create_task(self._poll_loop("folder", settings.folder_scan_minutes)),
        ]

    async def stop(self) -> None:
        for task in self._tasks:
            task.cancel()
        await asyncio.gather(*self._tasks, return_exceptions=True)

    async def _poll_loop(self, connector_type: str, minutes: int) -> None:
        while True:
            await self._sync_connector(connector_type)
            await asyncio.sleep(minutes * 60)

    async def _sync_connector(self, connector_type: str) -> None:
        with SessionLocal() as db:
            sources = list(
                db.execute(
                    select(Source).where(
                        Source.connector == connector_type, Source.enabled.is_(True)
                    )
                ).scalars()
            )
        for source in sources:
            try:
                with SessionLocal() as db:
                    await self.pipeline.sync_source(db, source)
            except Exception as exc:  # noqa: BLE001
                logger.warning("scheduled sync failed for %s: %s", source.name, exc)
