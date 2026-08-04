from sqlalchemy import create_engine, event
from sqlalchemy.orm import DeclarativeBase, sessionmaker

from .config import settings

# Short busy timeout: with no write lock held across awaits, contention windows
# are milliseconds. A long timeout would turn a conflict into a multi-second
# stall of the single event-loop thread (sqlite busy-wait is blocking).
connect_args = {"check_same_thread": False, "timeout": 5} if "sqlite" in settings.resolved_database_url else {}

engine = create_engine(settings.resolved_database_url, connect_args=connect_args)


@event.listens_for(engine, "connect")
def _load_sqlite_vec(dbapi_connection, _record):  # noqa: ANN001
    try:
        import sqlite_vec

        dbapi_connection.enable_load_extension(True)
        sqlite_vec.load(dbapi_connection)
        dbapi_connection.enable_load_extension(False)
    except ImportError:
        pass


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
