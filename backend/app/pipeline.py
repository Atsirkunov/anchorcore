import asyncio
import json
import logging
import time
from datetime import datetime, timezone

from sqlalchemy import delete, or_, select
from sqlalchemy.orm import Session

from .chunking import chunk_text, chunk_document, classify_windows
from .classifier import DISTILL_DOC_TYPES, Classifier
from .cleaning import clean_text, repeated_lines, strip_repeated
from .config import settings
from .connectors import IngestionDoc, build_connector
from .distill import signal, distill_and_store
from .embedder import Embedder, pack_f32
from .hashing import content_hash, window_hash
from .models import Chunk, Entity, IngestedItem, MergeAction, Relationship, Source
from .redact import redact
from .secrets import SecretStore, resolve_source_config
from .system_events import record as record_event
from .throughput import throughput

logger = logging.getLogger(__name__)


def record_sync_error(db: Session, source: Source, exc: Exception) -> None:
    source.last_error = redact(f"{type(exc).__name__}: {exc}")[:1000]
    source.error_count += 1
    db.commit()


def record_sync_success(db: Session, source: Source) -> None:
    source.last_error = None
    source.error_count = 0
    db.commit()


def _detach_entity_refs(db: Session, entity_ids: list[int]) -> None:
    """Delete merge_actions/relationships referencing entities about to be
    deleted (the ORM would otherwise try to NULL their NOT NULL FKs)."""
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
            record_event("pipeline", f"sync failed for source '{source.name}'",
                         source_id=source.id, detail=f"{type(exc).__name__}: {exc}", db=db)
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
                # Commit BEFORE classification: LLM calls are slow and must
                # never run while this session holds the SQLite write lock
                # (a blocked sqlite busy-wait stalls the whole event loop).
                db.commit()
                created_items += 1
                new_entities += await self._classify_and_store(db, source, doc, force_reclassify)

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

    async def _classify_and_store(
        self,
        db: Session,
        source: Source,
        doc: IngestionDoc,
        force_reclassify: bool = False,
    ) -> int:
        # LLM calls must never run while this session holds the SQLite write
        # lock, so classify BEFORE touching the DB (see commit-before-LLM
        # invariant in _sync_source).
        base_ref = doc.source_ref or doc.title
        windows = classify_windows(doc.text)

        item = db.execute(
            select(IngestedItem).where(
                IngestedItem.source_id == source.id,
                IngestedItem.external_id == doc.external_id,
            )
        ).scalar_one()

        # B39 PII gate: sensitive/pii sources must not reach a cloud provider
        # until the user confirms it (ANCHOR_CLOUD_TRUST=1). We still classify
        # locally with rules + embed locally, but refuse the remote LLM path.
        from .status import cloud_classifier_trusted, cloud_embedder_trusted, sensitive_label

        classify_trusted = cloud_classifier_trusted(self.classifier.settings)
        embed_trusted = cloud_embedder_trusted(self.classifier.settings)
        gated = sensitive_label(source.label) and not (classify_trusted and embed_trusted)

        # document-type pre-pass (B26): one cheap call per document, cached on
        # the item so reclassify doesn't re-detect
        doc_type = item.doc_type or "general"
        if not item.doc_type:
            doc_type = await self.classifier.detect_document_type(doc.text[:12000], cloud_trusted=classify_trusted)
            item.doc_type = doc_type
            db.commit()

        if gated:
            record_event(
                "pipeline",
                f"PII gate: source '{source.name}' skipped cloud provider (unconfirmed)",
                source_id=source.id,
                level="warning",
                detail=(
                    f"label={source.label}; classify_cloud={classify_trusted}, "
                    f"embed_cloud={embed_trusted}. Used rule-based classification "
                    "and local processing. Set ANCHOR_CLOUD_TRUST=1 to allow cloud."
                ),
                db=db,
            )

        # per-window hash-skip on incremental reclassify (cheaper cloud calls)
        prev_hashes = json.loads(item.window_hashes or "{}")
        sem = asyncio.Semaphore(settings.classifier_concurrency)

        if force_reclassify:
            # drop ALL entities for the item (full rebuild — also clears
            # pre-migration entities that have no window_index)
            old_all = db.execute(select(Entity).where(Entity.item_id == item.id)).scalars().all()
            _detach_entity_refs(db, [e.id for e in old_all])
            for entity in old_all:
                db.delete(entity)
            db.flush()

        async def classify_one(window: str, ref: str, index: int) -> list[dict] | None:
            h = window_hash(window)
            if not force_reclassify and prev_hashes.get(str(index)) == h:
                return None  # unchanged since last classification
            async with sem:
                started = time.monotonic()
                try:
                    classified = await self.classifier.classify(
                        window, ref, doc_type, cloud_trusted=classify_trusted
                    )
                    for entity_data in classified:
                        entity_data["window_text"] = window
                        entity_data["window_index"] = index
                    return classified
                finally:
                    throughput.record(time.monotonic() - started)

        refs = [f"{base_ref} §{index}" if len(windows) > 1 else base_ref for index in range(1, len(windows) + 1)]
        results = await asyncio.gather(
            *(classify_one(w, r, i) for i, (w, r) in enumerate(zip(windows, refs), start=1))
        )

        new_hashes = dict(prev_hashes)
        for index, (window, batch) in enumerate(zip(windows, results), start=1):
            new_hashes[str(index)] = window_hash(window)
            if batch is None:
                continue  # window unchanged — keep existing entities
            stale = db.execute(
                select(Entity).where(Entity.item_id == item.id, Entity.window_index == index)
            ).scalars().all()
            _detach_entity_refs(db, [e.id for e in stale])
            for entity in stale:
                db.delete(entity)
            db.flush()

            seen: set[str] = set()
            for item_data in batch:
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
                    window_text=item_data.get("window_text", "")[:4000],
                    window_index=item_data.get("window_index"),
                )
                db.add(entity)
                db.flush()

                for chunk_content in chunk_text(item_data["summary"]):
                    db.add(
                        Chunk(
                            entity_id=entity.id,
                            kind="entity",
                            source_ref=item_data["source_ref"],
                            content=chunk_content,
                        )
                    )

        item.window_hashes = json.dumps(new_hashes)

        # B18 distillation (chat-like doc types → Q&A units), hash-skipped.
        if settings.distill_enabled and doc_type in DISTILL_DOC_TYPES:
            await distill_and_store(
                self.classifier, db, item, doc, windows, force_reclassify, cloud_trusted=classify_trusted
            )

        # full-document chunks so long content is fully retrievable; clean
        # BEFORE chunking so boundaries/embeddings/FTS tokens are clean.
        # Rebuilt each pass — drop previous doc chunks so re-syncs don't dup.
        old_doc_chunks = db.execute(
            select(Chunk).where(Chunk.item_id == item.id, Chunk.entity_id.is_(None), Chunk.kind != "distilled")
        ).scalars().all()
        for chunk in old_doc_chunks:
            db.delete(chunk)
        db.flush()
        cleaned = strip_repeated(clean_text(doc.text), repeated_lines(doc.text))
        doc_chunks = chunk_document(cleaned)
        for index, chunk_content in enumerate(doc_chunks, start=1):
            ref = f"{base_ref} §{index}" if len(doc_chunks) > 1 else base_ref
            db.add(Chunk(item_id=item.id, kind="document", source_ref=ref, content=chunk_content))

        db.commit()

        self._flag_pii(db, item)
        await self._embed_item(db, item, cloud_trusted=embed_trusted, gated=gated)
        return sum(1 for batch in results if batch)

    def _flag_pii(self, db: Session, item: IngestedItem) -> None:
        """B30: scan the item's chunks for PII and set is_pii/pii_categories.
        Local regex scan (no LLM); strong matches auto-flag is_pii, the review
        page can confirm or override."""
        from .pii import load_config, scan_text

        chunks = db.execute(
            select(Chunk).where(
                or_(
                    Chunk.item_id == item.id,
                    Chunk.entity_id.in_(select(Entity.id).where(Entity.item_id == item.id)),
                )
            )
        ).scalars().all()
        cfg = load_config(db)
        for chunk in chunks:
            matches = scan_text(chunk.content, disabled=cfg["disabled"], custom_words=cfg["custom_words"])
            chunk.pii_categories = json.dumps([m.category for m in matches])
            chunk.is_pii = any(m.strong for m in matches)
        db.commit()

    async def _embed_item(
        self, db: Session, item: IngestedItem, cloud_trusted: bool = True, gated: bool = False
    ) -> None:
        chunks = db.execute(
            select(Chunk).where(
                or_(Chunk.item_id == item.id, Chunk.entity_id.in_(
                    select(Entity.id).where(Entity.item_id == item.id)
                ))
            )
        ).scalars().all()

        pending = [c for c in chunks if c.embedding is None]
        if not pending:
            return
        # B18 IDF gating: low-signal chunks (filler/rare-token sparse) stay in
        # FTS but are skipped from vector embedding.
        embeddable = [c for c in pending if signal(c.content, pending) >= settings.embed_min_signal]
        skipped = len(pending) - len(embeddable)
        if skipped:
            logger.info("IDF gate: skipping embeddings for %d low-signal chunk(s) of %s", skipped, item.title)
        texts = [c.content for c in embeddable]
        if not texts:
            return
        # B30: sensitive/pii sources AND PII-flagged chunks are only embedded
        # by a trusted (local) embedder; when the gate trips we skip remote
        # embedding and keep the chunks in FTS (keyword retrieval still works
        # locally).
        if gated and not cloud_trusted:
            logger.info(
                "embed gate: %d chunk(s) of %s not sent to remote embedder (PII gate)",
                len(texts),
                item.title,
            )
            return
        if not cloud_trusted:
            pii_chunks = [c for c in embeddable if c.is_pii]
            if pii_chunks:
                logger.info(
                    "embed gate: %d PII-flagged chunk(s) of %s not sent to remote embedder",
                    len(pii_chunks),
                    item.title,
                )
                embeddable = [c for c in embeddable if not c.is_pii]
                texts = [c.content for c in embeddable]
                if not texts:
                    return
        try:
            vectors = await self.embedder.embed(texts)
        except Exception as exc:  # noqa: BLE001
            logger.warning("embedding failed (%s); chunks stored unembedded", exc)
            return
        for chunk, vector in zip(embeddable, vectors):
            if chunk.embedding is None:
                chunk.embedding = pack_f32([vector])
        db.commit()
