import logging

from sqlalchemy import create_engine, event
from sqlalchemy.orm import DeclarativeBase, sessionmaker

from .config import settings

logger = logging.getLogger(__name__)

# Short busy timeout: with no write lock held across awaits, contention windows
# are milliseconds. A long timeout would turn a conflict into a multi-second
# stall of the single event-loop thread (sqlite busy-wait is blocking).
connect_args = {"check_same_thread": False, "timeout": 5} if "sqlite" in settings.resolved_database_url else {}

engine = create_engine(settings.resolved_database_url, connect_args=connect_args)


@event.listens_for(engine, "connect")
def _load_sqlite_vec(dbapi_connection, _record):  # noqa: ANN001
    try:
        import sqlite_vec
    except ImportError:
        logger.warning("sqlite-vec not installed; native vec SQL unavailable")
        return
    try:
        dbapi_connection.enable_load_extension(True)
        sqlite_vec.load(dbapi_connection)
        dbapi_connection.enable_load_extension(False)
    except AttributeError:
        # CPython built without loadable-extension support (e.g. the
        # GitHub-hosted-toolcache / python.org macOS builds: sqlite3 lacks
        # enable_load_extension). Vector retrieval runs pure-Python cosine,
        # so the app still works — but don't crash on startup like before.
        logger.warning(
            "this Python's sqlite3 has no enable_load_extension; "
            "native vec SQL unavailable (vector retrieval uses pure-Python "
            "cosine, so search still works). Rebuild with a Python compiled "
            "with loadable-extension support (e.g. Homebrew's) to enable it."
        )
    except RuntimeError as exc:  # noqa: BLE001
        logger.warning("sqlite-vec failed to load: %s; native vec SQL unavailable", exc)


@event.listens_for(engine, "connect")
def _sqlite_optimize(dbapi_connection, _record):  # noqa: ANN001
    """WAL allows concurrent readers + a single writer without instant
    'database is locked' failures when background jobs, the scheduler and
    request handlers all touch SQLite at once."""
    if "sqlite" not in settings.resolved_database_url:
        return
    cursor = dbapi_connection.cursor()
    try:
        cursor.execute("PRAGMA journal_mode=WAL")
        cursor.execute("PRAGMA busy_timeout=5000")
        cursor.execute("PRAGMA synchronous=NORMAL")
        # Without this, ondelete=... on FK columns never fires, and ORM
        # deletions that rely on it (source -> items -> entities) try to NULL
        # NOT NULL columns instead of cascading.
        cursor.execute("PRAGMA foreign_keys=ON")
    finally:
        cursor.close()


SessionLocal = sessionmaker(bind=engine, autocommit=False, autoflush=False)


class Base(DeclarativeBase):
    pass


def get_db():
    db = SessionLocal()
    try:
        yield db
    finally:
        db.close()
