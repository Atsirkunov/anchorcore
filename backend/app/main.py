import logging
from contextlib import asynccontextmanager
from pathlib import Path

from fastapi import FastAPI
from fastapi.middleware.cors import CORSMiddleware
from fastapi.staticfiles import StaticFiles

from .answer_engine import AnswerEngine
from .classifier import Classifier
from .config import settings
from .db import Base, SessionLocal, engine
from .embedder import Embedder
from .pipeline import IngestionPipeline
from .routers import entities, qa, sources
from .scheduler import Scheduler
from .secrets import SecretStore

logging.basicConfig(level=logging.INFO)
logger = logging.getLogger(__name__)

settings.data_dir.mkdir(parents=True, exist_ok=True)

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

app.include_router(sources.make_router(pipeline, secrets))
app.include_router(entities.make_router())
app.include_router(entities.review_router())
app.include_router(qa.make_router(answer_engine))


@app.get("/health")
def health() -> dict:
    return {"status": "ok", "data_dir": str(settings.data_dir)}


# Serve built frontend if present (must be last — catches everything else)
frontend_dist = Path(__file__).resolve().parents[2] / "frontend" / "dist"
if frontend_dist.exists():
    app.mount("/", StaticFiles(directory=frontend_dist, html=True), name="ui")
