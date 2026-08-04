import logging
import re
from datetime import datetime, timezone

from sqlalchemy import delete, or_, select
from sqlalchemy.orm import Session

from .classifier import Classifier
from .config import settings
from .connectors import ConnectorError, IngestionDoc, build_connector
from .embedder import Embedder, pack_f32
from .models import Chunk, Entity, IngestedItem, MergeAction, Relationship, Source, content_hash
from .redact import redact
from .secrets import SecretStore, resolve_source_config
from .system_events import record as record_event

logger = logging.getLogger(__name__)


def chunk_text(text: str) -> list[str]:
    """Fixed-size chunks with overlap (used for entity summaries)."""
    size, overlap = settings.chunk_size, settings.chunk_overlap
    if len(text) <= size:
        return [text]
    step = size - overlap
    return [text[i : i + size] for i in range(0, max(len(text) - size + 1, 1), step)]


def chunk_document(text: str) -> list[str]:
    """Section-aware chunks of a full document: split on paragraph boundaries,
    falling back to fixed-size with overlap for oversized paragraphs."""
    paragraphs = [p.strip() for p in re.split(r"\n\s*\n", text) if p.strip()]
    chunks: list[str] = []
    current = ""
    for paragraph in paragraphs:
        if len(paragraph) > settings.chunk_size:
            if current:
                chunks.append(current)
                current = ""
            chunks.extend(chunk_text(paragraph))
        elif len(current) + len(paragraph) + 2 <= settings.chunk_size:
            current = paragraph if not current else f"{current}\n\n{paragraph}"
        else:
            if current:
                chunks.append(current)
            current = paragraph
    if current:
        chunks.append(current)
    return chunks


def classify_windows(text: str) -> list[str]:
    """Split long documents into overlapping windows for classification."""
    window = settings.classify_window_chars
    if len(text) <= window:
        return [text]
    step = max(window // 2, 1)
    return [text[i : i + window] for i in range(0, max(len(text) - window + 1, 1), step)]


def record_sync_error(db: Session, source: Source, exc: Exception) -> None:
    source.last_error = redact(f"{type(exc).__name__}: {exc}")[:1000]
    source.error_count += 1
    db.commit()


def record_sync_success(db: Session, source: Source) -> None:
    source.last_error = None
    source.error_count = 0
    db.commit()


def _detach_entity_refs(db: Session, entity_ids: list[int]) -> None:
    """Delete rows that reference entities about to be deleted.

    The ORM would otherwise try to NULL their NOT NULL FK columns
    (merge_actions.entity_a_id, relationships.*_id), which fails.
    """
    if not entity_ids:
        return
    db.execute(
        delete(MergeAction).where(
            or_(MergeAction.entity_a_id.in_(entity_ids), MergeAction.entity_b_id.in_(entity_ids))
        )
    )
    db.execute(
        delete(Relationship).where(
            or_(Relationship.from_entity_id.in_(entity_ids), Relationship.to_entity_id.in_(entity_ids))
        )
    )


class IngestionPipeline:
    def __init__(self, classifier: Classifier, embedder: Embedder, secrets: SecretStore):
        self.classifier = classifier
        self.embedder = embedder
        self.secrets = secrets

    async def sync_source(
        self, db: Session, source: Source, force_reclassify: bool = False, progress=None
    ) -> dict:
        try:
            return await self._sync_source(db, source, force_reclassify, progress)
        except Exception as exc:
            record_sync_error(db, source, exc)
            record_event(
                "pipeline",
                f"sync failed for source '{source.name}'",
                source_id=source.id,
                detail=f"{type(exc).__name__}: {exc}",
                db=db,
            )
            raise

    async def _sync_source(
        self, db: Session, source: Source, force_reclassify: bool = False, progress=None
    ) -> dict:
        config = resolve_source_config(source, self.secrets)
        connector = build_connector(source.connector, config)
        docs, cursor = await connector.fetch(source.last_sync_cursor or "")

        created_items = 0
        new_entities = 0
        total = len(docs)
        for index, doc in enumerate(docs, start=1):
            if progress is not None:
                await progress(index, total)
            if self._upsert_doc(db, source, doc) or force_reclassify:
                # Commit the item BEFORE classification: LLM calls are slow and
                # must never run while this session holds the SQLite write lock
                # (other writers would block — and a blocked sqlite busy-wait
                # stalls the whole event loop).
                db.commit()
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
            missing_text = not existing.text
            if existing.content_hash == digest and not missing_text:
                return False
            existing.content_hash = digest
            existing.title = doc.title
            existing.author = doc.author
            existing.updated_at = doc.updated_at
            existing.stale = False
            if missing_text or existing.text != doc.text:
                existing.text = doc.text
            return True

        db.add(
            IngestedItem(
                source_id=source.id,
                external_id=doc.external_id,
                title=doc.title,
                text=doc.text,
                content_hash=digest,
                author=doc.author,
                updated_at=doc.updated_at,
            )
        )
        db.flush()
        return True

    async def _classify_and_store(self, db: Session, source: Source, doc: IngestionDoc) -> int:
        # Classify BEFORE touching the DB: LLM calls are slow and must never
        # hold a write lock on SQLite (blocks syncs/requests concurrently).
        base_ref = doc.source_ref or doc.title
        windows = classify_windows(doc.text)
        classified: list[dict] = []
        for index, window in enumerate(windows, start=1):
            ref = f"{base_ref} §{index}" if len(windows) > 1 else base_ref
            classified.extend(await self.classifier.classify(window, ref))

        item = db.execute(
            select(IngestedItem).where(
                IngestedItem.source_id == source.id,
                IngestedItem.external_id == doc.external_id,
            )
        ).scalar_one()

        # drop stale classifications from previous content, keep the item row
        old_entities = db.execute(select(Entity).where(Entity.item_id == item.id)).scalars().all()
        _detach_entity_refs(db, [e.id for e in old_entities])
        for entity in old_entities:
            db.delete(entity)
        db.flush()

        count = 0
        seen: set[str] = set()
        for item_data in classified:
            key = item_data["summary"].lower()
            if key in seen:
                continue
            seen.add(key)
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

        # full-document chunks so long content is fully retrievable
        doc_chunks = chunk_document(doc.text)
        for index, chunk_content in enumerate(doc_chunks, start=1):
            ref = f"{base_ref} §{index}" if len(doc_chunks) > 1 else base_ref
            db.add(Chunk(item_id=item.id, source_ref=ref, content=chunk_content))

        db.commit()

        await self._embed_item(db, item)
        return count

    async def _embed_item(self, db: Session, item: IngestedItem) -> None:
        chunks = db.execute(
            select(Chunk).where(
                or_(Chunk.item_id == item.id, Chunk.entity_id.in_(
                    select(Entity.id).where(Entity.item_id == item.id)
                ))
            )
        ).scalars().all()

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
