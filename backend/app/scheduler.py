import asyncio
import logging
from datetime import datetime, timezone

from sqlalchemy import select

from .config import settings
from .db import SessionLocal
from .embedder import pack_f32
from .folder_watcher import FolderWatcher
from .models import Chunk, Source

logger = logging.getLogger(__name__)


class Scheduler:
    """Background loops: periodic Jira polls, hourly folder re-scans, folder file watcher,
    embedding backfill, with a task watchdog that restarts dead loops."""

    def __init__(self, pipeline, embedder):
        self.pipeline = pipeline
        self.embedder = embedder
        self.watcher = FolderWatcher(settings.folder_watch_debounce)
        self._tasks: dict[str, asyncio.Task] = {}
        self._watch_task: asyncio.Task | None = None
        self._watchdog_task: asyncio.Task | None = None

    def start(self) -> None:
        self._tasks = {
            "jira": asyncio.create_task(self._poll_loop("jira", settings.jira_poll_minutes), name="jira"),
            "folder": asyncio.create_task(
                self._poll_loop("folder", settings.folder_scan_minutes), name="folder"
            ),
            "backfill": asyncio.create_task(self._backfill_loop(), name="backfill"),
        }
        self._watch_task = asyncio.create_task(self._watch_loop(), name="folder-watch")
        self._watchdog_task = asyncio.create_task(self._watchdog(), name="watchdog")
        self.watcher.start()
        self.reload_sources()
        asyncio.create_task(self._backfill_embeddings(), name="backfill-startup")

    async def stop(self) -> None:
        tasks = list(self._tasks.values()) + [self._watch_task, self._watchdog_task]
        tasks = [t for t in tasks if t is not None]
        for task in tasks:
            task.cancel()
        await asyncio.gather(*tasks, return_exceptions=True)
        self.watcher.stop()

    def task_states(self) -> dict[str, str]:
        states = {name: ("running" if not task.done() else "stopped") for name, task in self._tasks.items()}
        if self._watch_task is not None:
            states["folder-watch"] = "running" if not self._watch_task.done() else "stopped"
        return states

    async def _backfill_loop(self) -> None:
        while True:
            await asyncio.sleep(15 * 60)
            try:
                await self._backfill_embeddings()
            except Exception as exc:  # noqa: BLE001
                logger.error("embedding backfill loop crashed: %s", exc)

    async def _backfill_embeddings(self) -> None:
        with SessionLocal() as db:
            chunks = list(db.execute(select(Chunk).where(Chunk.embedding.is_(None))).scalars())
            if not chunks:
                return
            texts = [c.content for c in chunks]
            try:
                vectors = await self.embedder.embed(texts)
            except Exception as exc:  # noqa: BLE001
                logger.warning("embedding backfill failed (%s); %d chunks still pending", exc, len(chunks))
                return
            for chunk, vector in zip(chunks, vectors):
                chunk.embedding = pack_f32([vector])
            db.commit()
            logger.info("embedding backfill: embedded %d chunks", len(chunks))

    def reload_sources(self) -> None:
        """(Re)build folder watchers from enabled folder sources in the DB."""
        with SessionLocal() as db:
            folder_sources = list(
                db.execute(
                    select(Source).where(Source.connector == "folder", Source.enabled.is_(True))
                ).scalars()
            )
        current = {s.id for s in folder_sources}
        for source_id in list(self.watcher.current_ids()):
            if source_id not in current:
                self.watcher.remove(source_id)
        for source in folder_sources:
            path = source.config_dict().get("path")
            if path:
                self.watcher.add(source.id, path)

    async def _watchdog(self) -> None:
        """Restart any poll task that died unexpectedly."""
        while True:
            await asyncio.sleep(30)
            for name, task in list(self._tasks.items()):
                if task.done() and not task.cancelled():
                    exc = task.exception()
                    logger.error("task %s died (%s); restarting", name, exc)
                    self._tasks[name] = asyncio.create_task(self._poll_loop(name, _minutes_for(name)), name=name)

    async def _watch_loop(self) -> None:
        while True:
            await asyncio.sleep(0.5)
            for source_id in self.watcher.drain():
                await self._sync_source_by_id(source_id)

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
            await self._sync_source_by_id(source.id)

    async def _sync_source_by_id(self, source_id: int) -> None:
        try:
            with SessionLocal() as db:
                source = db.get(Source, source_id)
                if source is None:
                    return
                await self.pipeline.sync_source(db, source)
        except Exception as exc:  # noqa: BLE001
            logger.warning("sync failed for source %s: %s", source_id, exc)


def _minutes_for(connector_type: str) -> int:
    return settings.jira_poll_minutes if connector_type == "jira" else settings.folder_scan_minutes
