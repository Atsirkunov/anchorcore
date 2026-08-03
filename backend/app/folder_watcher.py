import logging
import queue
import time
from pathlib import Path

from watchdog.events import FileSystemEventHandler
from watchdog.observers import Observer

logger = logging.getLogger(__name__)

SUPPORTED_SUFFIXES = {".txt", ".md", ".markdown", ".text", ".csv", ".json", ".pdf", ".html", ".htm"}


class _SyncHandler(FileSystemEventHandler):
    def __init__(self, source_id: int, sink: queue.Queue):
        self.source_id = source_id
        self._sink = sink

    def _relevant(self, path: str) -> bool:
        return Path(path).suffix.lower() in SUPPORTED_SUFFIXES

    def on_created(self, event):
        if not event.is_directory and self._relevant(event.src_path):
            self._sink.put((self.source_id, time.monotonic()))

    def on_modified(self, event):
        if not event.is_directory and self._relevant(event.src_path):
            self._sink.put((self.source_id, time.monotonic()))

    def on_moved(self, event):
        if not event.is_directory and self._relevant(event.dest_path):
            self._sink.put((self.source_id, time.monotonic()))

    def on_deleted(self, event):
        if not event.is_directory and self._relevant(event.src_path):
            self._sink.put((self.source_id, time.monotonic()))


class FolderWatcher:
    """Filesystem watcher that surfaces 'this source changed' events to the event loop.

    Events are pushed into a thread-safe queue; the scheduler drains it and
    syncs only sources whose last event is older than the debounce window
    (bursty writes coalesce into one sync).
    """

    def __init__(self, debounce_seconds: float = 3.0):
        self._observer: Observer | None = None
        self._sink: queue.Queue[tuple[int, float]] = queue.Queue()
        self._handlers: dict[int, _SyncHandler] = {}
        self._debounce_seconds = debounce_seconds
        self._pending: dict[int, float] = {}

    def _ensure_observer(self) -> Observer:
        if self._observer is None:
            self._observer = Observer()
        return self._observer

    def add(self, source_id: int, path: str) -> bool:
        path_obj = Path(path).expanduser().resolve()
        if not path_obj.is_dir():
            logger.warning("watcher: folder not found for source %s: %s", source_id, path)
            return False
        handler = _SyncHandler(source_id, self._sink)
        self._ensure_observer().schedule(handler, str(path_obj), recursive=True)
        self._handlers[source_id] = handler
        logger.info("watcher: watching %s for source %s", path_obj, source_id)
        return True

    def remove(self, source_id: int) -> None:
        handler = self._handlers.pop(source_id, None)
        if handler is not None and self._observer is not None:
            self._observer.unschedule(handler)

    def current_ids(self) -> set[int]:
        return set(self._handlers)

    def start(self) -> None:
        observer = self._ensure_observer()
        if observer.is_alive():
            return
        observer.start()

    def stop(self) -> None:
        if self._observer is not None:
            self._observer.stop()
            self._observer.join(timeout=5)
            self._observer = None

    def drain(self) -> set[int]:
        """Source ids whose last event is older than the debounce window.

        Events too recent to fire yet stay pending; each source fires at most
        once per drain window (bursts coalesce into a single sync).
        """
        while True:
            try:
                source_id, ts = self._sink.get_nowait()
            except queue.Empty:
                break
            self._pending[source_id] = max(self._pending.get(source_id, 0.0), ts)

        now = time.monotonic()
        due = {sid for sid, ts in self._pending.items() if now - ts >= self._debounce_seconds}
        for sid in due:
            del self._pending[sid]
        return due
