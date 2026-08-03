import logging
from datetime import datetime, timezone

from sqlalchemy import select
from sqlalchemy.orm import Session

from .classifier import Classifier
from .config import settings
from .connectors import ConnectorError, IngestionDoc, build_connector
from .embedder import Embedder, pack_f32
from .models import Chunk, Entity, IngestedItem, Source, content_hash
from .secrets import SecretStore, resolve_source_config

logger = logging.getLogger(__name__)


def chunk_text(text: str) -> list[str]:
    size, overlap = settings.chunk_size, settings.chunk_overlap
    if len(text) <= size:
        return [text]
    step = size - overlap
    return [text[i : i + size] for i in range(0, max(len(text) - size + 1, 1), step)]


def record_sync_error(db: Session, source: Source, exc: Exception) -> None:
    source.last_error = f"{type(exc).__name__}: {exc}"[:1000]
    source.error_count += 1
    db.commit()


def record_sync_success(db: Session, source: Source) -> None:
    source.last_error = None
    source.error_count = 0
    db.commit()


class IngestionPipeline:
    def __init__(self, classifier: Classifier, embedder: Embedder, secrets: SecretStore):
        self.classifier = classifier
        self.embedder = embedder
        self.secrets = secrets

    async def sync_source(self, db: Session, source: Source) -> dict:
        try:
            return await self._sync_source(db, source)
        except Exception as exc:
            record_sync_error(db, source, exc)
            raise

    async def _sync_source(self, db: Session, source: Source) -> dict:
        config = resolve_source_config(source, self.secrets)
        connector = build_connector(source.connector, config)
        docs, cursor = await connector.fetch(source.last_sync_cursor or "")

        created_items = 0
        new_entities = 0
        for doc in docs:
            if self._upsert_doc(db, source, doc):
                created_items += 1
                new_entities += await self._classify_and_store(db, source, doc)

        if cursor:
            source.last_sync_cursor = cursor
        source.last_synced_at = datetime.now(timezone.utc)
        record_sync_success(db, source)
        logger.info("source %s synced: %d items, %d entities", source.name, created_items, new_entities)
        return {"items": created_items, "entities": new_entities}

    def _upsert_doc(self, db: Session, source: Source, doc: IngestionDoc) -> bool:
        digest = content_hash(doc.text)
        existing = db.execute(
            select(IngestedItem).where(
                IngestedItem.source_id == source.id,
                IngestedItem.external_id == doc.external_id,
            )
        ).scalar_one_or_none()

        if existing is not None:
            if existing.content_hash == digest:
                return False
            existing.content_hash = digest
            existing.title = doc.title
            existing.author = doc.author
            existing.updated_at = doc.updated_at
            existing.stale = False
            return True

        db.add(
            IngestedItem(
                source_id=source.id,
                external_id=doc.external_id,
                title=doc.title,
                content_hash=digest,
                author=doc.author,
                updated_at=doc.updated_at,
            )
        )
        db.flush()
        return True

    async def _classify_and_store(self, db: Session, source: Source, doc: IngestionDoc) -> int:
        item = db.execute(
            select(IngestedItem).where(
                IngestedItem.source_id == source.id,
                IngestedItem.external_id == doc.external_id,
            )
        ).scalar_one()

        # drop stale classifications from previous content, keep the item row
        old_entities = db.execute(select(Entity).where(Entity.item_id == item.id)).scalars().all()
        for entity in old_entities:
            db.delete(entity)
        db.flush()

        classified = await self.classifier.classify(doc.text, doc.source_ref or doc.title)
        count = 0
        for item_data in classified:
            entity = Entity(
                item_id=item.id,
                kind=item_data["kind"],
                summary=item_data["summary"],
                reasoning=item_data["reasoning"],
                confidence=item_data["confidence"],
                author=item_data["author"] or doc.author,
                source_ref=item_data["source_ref"],
            )
            db.add(entity)
            db.flush()

            for chunk_content in chunk_text(item_data["summary"]):
                db.add(
                    Chunk(
                        entity_id=entity.id,
                        source_ref=item_data["source_ref"],
                        content=chunk_content,
                    )
                )
            count += 1

        db.commit()

        await self._embed_item(db, item)
        return count

    async def _embed_item(self, db: Session, item: IngestedItem) -> None:
        chunks = db.execute(select(Chunk).where(Chunk.entity_id.in_(
            select(Entity.id).where(Entity.item_id == item.id)
        ))).scalars().all()

        texts = [c.content for c in chunks if c.embedding is None]
        if not texts:
            return
        try:
            vectors = await self.embedder.embed(texts)
        except Exception as exc:  # noqa: BLE001
            logger.warning("embedding failed (%s); chunks stored unembedded", exc)
            return
        for chunk, vector in zip(chunks, vectors):
            if chunk.embedding is None:
                chunk.embedding = pack_f32([vector])
        db.commit()
