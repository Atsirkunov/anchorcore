import asyncio
import json
import logging
import re
import time
from datetime import datetime, timezone

from sqlalchemy import delete, or_, select
from sqlalchemy.orm import Session

from .classifier import Classifier, window_hash
from .cleaning import clean_text, repeated_lines, strip_repeated
from .config import settings
from .connectors import ConnectorError, IngestionDoc, build_connector
from .embedder import Embedder, pack_f32
from .models import Chunk, Entity, IngestedItem, MergeAction, Relationship, Source, content_hash
from .redact import redact
from .secrets import SecretStore, resolve_source_config
from .system_events import record as record_event
from .throughput import throughput

logger = logging.getLogger(__name__)


def chunk_text(text: str) -> list[str]:
    """Fixed-size chunks with overlap (used for entity summaries)."""
    size, overlap = settings.chunk_size, settings.chunk_overlap
    if len(text) <= size:
        return [text]
    step = size - overlap
    return [text[i : i + size] for i in range(0, max(len(text) - size + 1, 1), step)]


_HEADING_RE = re.compile(
    r"^\s*(?:"
    r"§\s*\d+(\.\d+)*"  # §434, §4.2.1
    r"|\d{1,4}(\.\d{1,4}){1,3}"  # 4.2.1, 12.3.4.5
    r"|(?:article|annex|section|schedule|rule|appendix)\s+\d+"  # Article 12
    r")(?:\s|[:.)\-]|$)",
    re.IGNORECASE,
)

_CAPS_HEADING_RE = re.compile(r"^[A-Z][A-Z0-9 &()/\-]{3,80}$")


def _is_heading(line: str) -> bool:
    """A line that looks like a section heading: numbered markers (§434,
    4.2.1, Article 12) or short ALL-CAPS titles. Page-number lines are
    already removed by cleaning before chunking."""
    stripped = line.strip()
    if not stripped or len(stripped) > 80:
        return False
    if _HEADING_RE.match(stripped):
        return True
    if _CAPS_HEADING_RE.match(stripped) and not stripped.isdigit():
        return True
    return False


def chunk_document(text: str) -> list[str]:
    """Section-aware chunks: hard boundaries at heading lines, paragraph
    accumulation inside a section, fixed-size fallback only for oversized
    paragraphs/sections. Max section size = chunk_max_chars."""
    max_chars = settings.chunk_max_chars
    lines = text.split("\n")
    sections: list[str] = []
    current: list[str] = []
    for line in lines:
        if _is_heading(line):
            if current:
                sections.append("\n".join(current))
                current = []
            current.append(line)
        else:
            current.append(line)
    if current:
        sections.append("\n".join(current))

    chunks: list[str] = []
    for section in sections:
        chunks.extend(_split_section(section, max_chars))
    return chunks


def _split_section(section: str, max_chars: int) -> list[str]:
    """Split one section by paragraphs; fall back to fixed-size for oversized
    paragraphs. Keeps headings attached to their section's first chunk."""
    if len(section) <= max_chars:
        return [section]
    paragraphs = [p.strip() for p in re.split(r"\n\s*\n", section) if p.strip()]
    out: list[str] = []
    current = ""
    for paragraph in paragraphs:
        if len(paragraph) > max_chars:
            if current:
                out.append(current)
                current = ""
            out.extend(chunk_text(paragraph))
        elif len(current) + len(paragraph) + 2 <= max_chars:
            current = paragraph if not current else f"{current}\n\n{paragraph}"
        else:
            if current:
                out.append(current)
            current = paragraph
    if current:
        out.append(current)
    return out


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
        # Classify BEFORE touching the DB: LLM calls are slow and must never
        # hold a write lock on SQLite (blocks syncs/requests concurrently).
        base_ref = doc.source_ref or doc.title
        windows = classify_windows(doc.text)

        item = db.execute(
            select(IngestedItem).where(
                IngestedItem.source_id == source.id,
                IngestedItem.external_id == doc.external_id,
            )
        ).scalar_one()

        # document-type pre-pass (B26): one cheap call per document, cached on
        # the item so reclassify doesn't re-detect
        doc_type = item.doc_type or "general"
        if not item.doc_type:
            doc_type = await self.classifier.detect_document_type(doc.text[:12000])
            item.doc_type = doc_type
            db.commit()

        # per-window: skip unchanged windows on reclassify (cheaper cloud calls).
        # force_reclassify → classify everything; incremental (sync) → skip
        # windows whose content hash is unchanged since last run.
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
                    classified = await self.classifier.classify(window, ref, doc_type)
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
            # drop entities that came from this window, keep the rest
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
                            source_ref=item_data["source_ref"],
                            content=chunk_content,
                        )
                    )

        item.window_hashes = json.dumps(new_hashes)

        # full-document chunks so long content is fully retrievable;
        # clean BEFORE chunking so boundaries/embeddings/FTS tokens are clean.
        # Rebuild them each pass (hash-skip applies to classification only) —
        # drop the previous doc chunks first so re-syncs don't duplicate.
        old_doc_chunks = db.execute(
            select(Chunk).where(Chunk.item_id == item.id, Chunk.entity_id.is_(None))
        ).scalars().all()
        for chunk in old_doc_chunks:
            db.delete(chunk)
        db.flush()
        cleaned = strip_repeated(clean_text(doc.text), repeated_lines(doc.text))
        doc_chunks = chunk_document(cleaned)
        for index, chunk_content in enumerate(doc_chunks, start=1):
            ref = f"{base_ref} §{index}" if len(doc_chunks) > 1 else base_ref
            db.add(Chunk(item_id=item.id, source_ref=ref, content=chunk_content))

        db.commit()

        await self._embed_item(db, item)
        return sum(1 for batch in results if batch)

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
