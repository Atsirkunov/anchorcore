from sqlalchemy import create_engine, event
from sqlalchemy.orm import DeclarativeBase, sessionmaker

from .config import settings

connect_args = {"check_same_thread": False, "timeout": 30} if "sqlite" in settings.resolved_database_url else {}

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


SessionLocal = sessionmaker(bind=engine, autocommit=False, autoflush=False)


class Base(DeclarativeBase):
    pass


def get_db():
    db = SessionLocal()
    try:
        yield db
    finally:
        db.close()
