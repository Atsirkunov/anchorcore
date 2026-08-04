import logging
from contextlib import asynccontextmanager
from logging.handlers import RotatingFileHandler
from pathlib import Path

import httpx
from fastapi import Depends, FastAPI
from fastapi.middleware.cors import CORSMiddleware
from fastapi.staticfiles import StaticFiles
from sqlalchemy import func, inspect, select
from sqlalchemy.orm import Session

from .answer_engine import AnswerEngine
from .classifier import Classifier
from .config import settings, validate_env_file
from .db import engine, get_db
from .embedder import Embedder
from .jobs import JobManager
from .models import Chunk, Source
from .pipeline import IngestionPipeline
from .redact import RedactingFormatter
from .routers import entities, qa, sources, system
from .scheduler import Scheduler
from .secrets import SecretStore
from .status import answer_provider, missing_ollama_models, ollama_reachable

settings.data_dir.mkdir(parents=True, exist_ok=True)


def _setup_logging() -> None:
    formatter = RedactingFormatter("%(asctime)s %(levelname)s %(name)s %(message)s")
    handler = RotatingFileHandler(
        settings.data_dir / "anchorcore.log",
        maxBytes=10_000_000,
        backupCount=3,
        encoding="utf-8",
    )
    handler.setFormatter(formatter)
    logging.getLogger().addHandler(handler)


logging.basicConfig(level=logging.INFO)
_setup_logging()
for handler in logging.getLogger().handlers:
    if isinstance(handler, logging.StreamHandler):
        handler.setFormatter(RedactingFormatter("%(levelname)s %(name)s %(message)s"))
logger = logging.getLogger(__name__)

for warning in validate_env_file():
    logger.warning("%s", warning)

classifier = Classifier()
embedder = Embedder()
secrets = SecretStore(settings.data_dir / "secrets.enc")
pipeline = IngestionPipeline(classifier, embedder, secrets)
answer_engine = AnswerEngine(embedder)
scheduler = Scheduler(pipeline, embedder)
jobs = JobManager(pipeline)


def _run_migrations() -> None:
    """Apply schema migrations via Alembic.

    Legacy databases created before Alembic was introduced have the base
    schema but no alembic_version table — stamp them at the BASE revision so
    `upgrade head` only creates tables added since (e.g. jobs) instead of
    failing on CREATE TABLE for the legacy ones.
    """
    from alembic import command
    from alembic.config import Config as AlembicConfig
    from alembic.script import ScriptDirectory

    backend_dir = Path(__file__).resolve().parents[1]
    cfg = AlembicConfig(str(backend_dir / "alembic.ini"))
    cfg.set_main_option("script_location", str(backend_dir / "alembic"))
    cfg.set_main_option("sqlalchemy.url", settings.resolved_database_url)
    with engine.connect() as conn:
        tables = inspect(conn).get_table_names()
    if "sources" in tables and "alembic_version" not in tables:
        base = ScriptDirectory.from_config(cfg).get_base()
        command.stamp(cfg, base.revision)
    command.upgrade(cfg, "head")


@asynccontextmanager
async def lifespan(_app: FastAPI):
    _run_migrations()
    scheduler.start()
    for line in settings.banner():
        logger.info("%s", line)
    missing = missing_ollama_models()
    if missing:
        logger.warning(
            "Ollama models missing: %s — classification falls back to rule-based until pulled. "
            "Run: ollama pull %s",
            ", ".join(missing),
            " && ollama pull ".join(missing),
        )
    logger.info("AnchorCore started. Data dir: %s", settings.data_dir)
    yield
    await scheduler.stop()


app = FastAPI(title="AnchorCore", version="1.0.0", lifespan=lifespan)

origins = [o.strip() for o in settings.cors_origins.split(",") if o.strip()]
app.add_middleware(
    CORSMiddleware,
    allow_origins=origins,
    allow_methods=["*"],
    allow_headers=["*"],
)

app.include_router(sources.make_router(pipeline, secrets, scheduler, jobs))
app.include_router(entities.make_router())
app.include_router(entities.review_router())
app.include_router(qa.make_router(answer_engine))
app.include_router(system.make_router(scheduler))


@app.get("/health")
async def health(db: Session = Depends(get_db)) -> dict:
    ollama_ok = ollama_reachable()
    failing_sources = list(
        db.execute(
            select(Source).where(Source.error_count > 0, Source.enabled.is_(True))
        ).scalars()
    )
    pending_embeddings = db.execute(
        select(func.count()).select_from(Chunk).where(Chunk.embedding.is_(None))
    ).scalar_one()
    return {
        "status": "ok",
        "data_dir": str(settings.data_dir),
        "components": {
            "ollama": "ok" if ollama_ok else "offline",
            "answer_key": answer_provider(),
            "pending_embeddings": pending_embeddings,
            "tasks": scheduler.task_states(),
            "failing_sources": [{"id": s.id, "name": s.name, "error": s.last_error, "count": s.error_count} for s in failing_sources],
        },
    }


# Serve built frontend if present (must be last — catches everything else)
frontend_dist = Path(__file__).resolve().parents[2] / "frontend" / "dist"
if frontend_dist.exists():
    app.mount("/", StaticFiles(directory=frontend_dist, html=True), name="ui")
