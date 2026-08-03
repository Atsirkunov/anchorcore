import asyncio
import logging
from datetime import datetime, timezone

from sqlalchemy import select

from .config import settings
from .db import SessionLocal
from .models import Source

logger = logging.getLogger(__name__)


class Scheduler:
    """Background loop: periodic Jira polls + hourly folder re-scans, with a task watchdog."""

    def __init__(self, pipeline):
        self.pipeline = pipeline
        self._tasks: dict[str, asyncio.Task] = {}

    def start(self) -> None:
        self._tasks = {
            "jira": asyncio.create_task(self._poll_loop("jira", settings.jira_poll_minutes), name="jira"),
            "folder": asyncio.create_task(
                self._poll_loop("folder", settings.folder_scan_minutes), name="folder"
            ),
        }
        asyncio.create_task(self._watchdog(), name="watchdog")

    async def stop(self) -> None:
        for task in list(self._tasks.values()):
            task.cancel()
        await asyncio.gather(*self._tasks.values(), return_exceptions=True)

    def task_states(self) -> dict[str, str]:
        return {name: ("running" if not task.done() else "stopped") for name, task in self._tasks.items()}

    async def _watchdog(self) -> None:
        """Restart any poll task that died unexpectedly."""
        while True:
            await asyncio.sleep(30)
            for name, task in list(self._tasks.items()):
                if task.done() and not task.cancelled():
                    exc = task.exception()
                    logger.error("task %s died (%s); restarting", name, exc)
                    self._tasks[name] = asyncio.create_task(self._poll_loop(name, _minutes_for(name)), name=name)

    async def _poll_loop(self, connector_type: str, minutes: int) -> None:
        while True:
            try:
                await self._sync_connector(connector_type)
            except Exception as exc:  # noqa: BLE001
                logger.error("connector loop %s crashed: %s", connector_type, exc)
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


def _minutes_for(connector_type: str) -> int:
    return settings.jira_poll_minutes if connector_type == "jira" else settings.folder_scan_minutes
