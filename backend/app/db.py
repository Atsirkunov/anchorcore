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


def migrate() -> None:
    """Lightweight additive migrations for existing SQLite databases."""
    if "sqlite" not in settings.resolved_database_url:
        return
    with engine.begin() as conn:
        chunk_cols = {row[1]: row for row in conn.exec_driver_sql("PRAGMA table_info(chunks)")}
        if chunk_cols.get("entity_id") and chunk_cols["entity_id"][3]:
            # old schema: entity_id NOT NULL -> rebuild with nullable cols
            _rebuild_chunks(conn)

        for table, columns in (
            ("ingested_items", [("text", "TEXT DEFAULT ''")]),
            ("chunks", [("item_id", "INTEGER")]),
        ):
            existing = {row[1] for row in conn.exec_driver_sql(f"PRAGMA table_info({table})")}
            for column, ddl in columns:
                if column not in existing:
                    conn.exec_driver_sql(f"ALTER TABLE {table} ADD COLUMN {column} {ddl}")


def _rebuild_chunks(conn) -> None:
    conn.exec_driver_sql(
        """
        CREATE TABLE chunks_new (
            id INTEGER NOT NULL PRIMARY KEY,
            item_id INTEGER,
            entity_id INTEGER,
            source_ref VARCHAR(500) NOT NULL DEFAULT '',
            content TEXT NOT NULL DEFAULT '',
            embedding BLOB,
            created_at DATETIME NOT NULL,
            FOREIGN KEY(item_id) REFERENCES ingested_items (id) ON DELETE CASCADE,
            FOREIGN KEY(entity_id) REFERENCES entities (id) ON DELETE CASCADE
        )
        """
    )
    conn.exec_driver_sql(
        """
        INSERT INTO chunks_new (id, item_id, entity_id, source_ref, content, embedding, created_at)
        SELECT id, item_id, entity_id, source_ref, content, embedding, created_at FROM chunks
        """
    )
    conn.exec_driver_sql("DROP TABLE chunks")
    conn.exec_driver_sql("ALTER TABLE chunks_new RENAME TO chunks")
