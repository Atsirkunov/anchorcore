import sys
from logging.config import fileConfig
from pathlib import Path

from alembic import context
from sqlalchemy import event, pool

# Make `app` importable regardless of cwd (backend dir is the script location root).
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from app.config import settings  # noqa: E402
from app import models  # noqa: E402, F401  (register all tables on Base.metadata)
from app.db import Base  # noqa: E402

config = context.config

# Skip alembic's own logging config (configure_logging = false in alembic.ini):
# running migrations inside the app startup must not replace the app's root
# handlers (rotating file log) or levels.
if config.config_file_name is not None and config.get_main_option("configure_logging", "true") != "false":
    fileConfig(config.config_file_name)

target_metadata = Base.metadata


def _database_url() -> str:
    # Single source of truth: app settings (env vars + .env) decide the URL.
    return config.get_main_option("sqlalchemy.url") or settings.resolved_database_url


def run_migrations_offline() -> None:
    """Run migrations in 'offline' mode."""
    context.configure(
        url=_database_url(),
        target_metadata=target_metadata,
        literal_binds=True,
        dialect_opts={"paramstyle": "named"},
    )

    with context.begin_transaction():
        context.run_migrations()


def run_migrations_online() -> None:
    """Run migrations in 'online' mode."""
    connectable = _make_engine()

    with connectable.connect() as connection:
        context.configure(connection=connection, target_metadata=target_metadata)

        with context.begin_transaction():
            context.run_migrations()


def _make_engine():
    from sqlalchemy import create_engine

    url = _database_url()
    connect_args = {"check_same_thread": False, "timeout": 30} if url.startswith("sqlite") else {}
    engine = create_engine(url, poolclass=pool.NullPool, connect_args=connect_args)
    if url.startswith("sqlite"):
        # Load sqlite-vec on every migration connection so the vec0 virtual
        # table (B33) can be created. Mirrors app/db.py's connect listener;
        # the migration itself degrades gracefully when the extension is
        # unavailable (Python without loadable extensions).
        from app.db import _load_sqlite_vec

        event.listen(engine, "connect", _load_sqlite_vec)
    return engine


if context.is_offline_mode():
    run_migrations_offline()
else:
    run_migrations_online()
