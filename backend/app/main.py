import logging
from contextlib import asynccontextmanager
from logging.handlers import RotatingFileHandler
from pathlib import Path

import httpx
from fastapi import Depends, FastAPI
from fastapi.middleware.cors import CORSMiddleware
from fastapi.staticfiles import StaticFiles
from sqlalchemy import select
from sqlalchemy.orm import Session

from .answer_engine import AnswerEngine
from .classifier import Classifier
from .config import settings
from .db import Base, SessionLocal, engine, get_db
from .embedder import Embedder
from .models import Source
from .pipeline import IngestionPipeline
from .routers import entities, qa, sources
from .scheduler import Scheduler
from .secrets import SecretStore

settings.data_dir.mkdir(parents=True, exist_ok=True)


def _setup_logging() -> None:
    handler = RotatingFileHandler(
        settings.data_dir / "anchorcore.log",
        maxBytes=10_000_000,
        backupCount=3,
        encoding="utf-8",
    )
    handler.setFormatter(logging.Formatter("%(asctime)s %(levelname)s %(name)s %(message)s"))
    logging.getLogger().addHandler(handler)


logging.basicConfig(level=logging.INFO)
_setup_logging()
logger = logging.getLogger(__name__)

classifier = Classifier()
embedder = Embedder()
secrets = SecretStore(settings.data_dir / "secrets.enc")
pipeline = IngestionPipeline(classifier, embedder, secrets)
answer_engine = AnswerEngine(embedder)
scheduler = Scheduler(pipeline)


@asynccontextmanager
async def lifespan(_app: FastAPI):
    Base.metadata.create_all(bind=engine)
    scheduler.start()
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

app.include_router(sources.make_router(pipeline, secrets, scheduler))
app.include_router(entities.make_router())
app.include_router(entities.review_router())
app.include_router(qa.make_router(answer_engine))


@app.get("/health")
async def health(db: Session = Depends(get_db)) -> dict:
    ollama_ok = await _check_ollama()
    failing_sources = list(
        db.execute(
            select(Source).where(Source.error_count > 0, Source.enabled.is_(True))
        ).scalars()
    )
    return {
        "status": "ok",
        "data_dir": str(settings.data_dir),
        "components": {
            "ollama": "ok" if ollama_ok else "offline",
            "answer_key": _answer_provider(),
            "tasks": scheduler.task_states(),
            "failing_sources": [{"id": s.id, "name": s.name, "error": s.last_error, "count": s.error_count} for s in failing_sources],
        },
    }


def _answer_provider() -> str:
    if settings.answer_base_url.startswith(("http://localhost", "http://127.0.0.1")):
        return "ollama"
    if settings.answer_api_key:
        return "configured"
    return "missing"


async def _check_ollama() -> bool:
    try:
        async with httpx.AsyncClient(timeout=httpx.Timeout(3.0, connect=2.0)) as client:
            resp = await client.get(f"{settings.ollama_base_url.rstrip('/')}/api/tags")
            return resp.status_code == 200
    except httpx.TransportError:
        return False


# Serve built frontend if present (must be last — catches everything else)
frontend_dist = Path(__file__).resolve().parents[2] / "frontend" / "dist"
if frontend_dist.exists():
    app.mount("/", StaticFiles(directory=frontend_dist, html=True), name="ui")
