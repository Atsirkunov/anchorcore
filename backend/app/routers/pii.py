"""PII configuration + review (B30 partial).

GET  /pii/config            — category catalog + custom filter words
PUT  /pii/config            — update custom words / disabled categories
GET  /pii/review            — chunks matched by the scan (optionally unreviewed)
POST /pii/review/{id}       — confirm/override a chunk's is_pii classification
POST /pii/scan/{source_id}  — (re)scan a source's chunks after config changes
"""

import json
import logging

from fastapi import APIRouter, Depends, HTTPException
from sqlalchemy import or_, select
from sqlalchemy.orm import Session

from .. import schemas
from ..db import get_db
from ..models import Chunk, Source
from ..pii import (
    categories_payload,
    load_config,
    save_config,
    scan_text,
    serialize_matches,
)

logger = logging.getLogger(__name__)

REVIEW_PAGE = 50


def make_router() -> APIRouter:
    router = APIRouter(prefix="/pii", tags=["pii"])

    @router.get("/config", response_model=schemas.PiiConfigOut)
    def get_config(db: Session = Depends(get_db)) -> dict:
        cfg = load_config(db)
        return {
            "categories": categories_payload(cfg["disabled"]),
            "custom_words": sorted(cfg["custom_words"]),
        }

    @router.put("/config", response_model=schemas.PiiConfigOut)
    def update_config(payload: schemas.PiiConfigUpdate, db: Session = Depends(get_db)) -> dict:
        save_config(
            db,
            disabled=payload.disabled_categories,
            custom_words=payload.custom_words,
        )
        cfg = load_config(db)
        return {
            "categories": categories_payload(cfg["disabled"]),
            "custom_words": sorted(cfg["custom_words"]),
        }

    @router.get("/review", response_model=list[schemas.PiiChunkOut])
    def review(
        only_flagged: bool = True,
        limit: int = REVIEW_PAGE,
        offset: int = 0,
        db: Session = Depends(get_db),
    ) -> list[dict]:
        cfg = load_config(db)
        disabled = cfg["disabled"]
        custom_words = cfg["custom_words"]

        from ..models import Entity, IngestedItem

        # a chunk reaches its source via its item (directly OR through its
        # entity's item) — mirror answer_engine._chunk_query's join
        query = (
            select(Chunk, Source)
            .outerjoin(Entity, Chunk.entity_id == Entity.id)
            .outerjoin(
                IngestedItem,
                or_(Chunk.item_id == IngestedItem.id, Entity.item_id == IngestedItem.id),
            )
            .outerjoin(Source, IngestedItem.source_id == Source.id)
        )
        rows = db.execute(query.order_by(Chunk.created_at.desc()).limit(limit).offset(offset)).all()

        out: list[dict] = []
        for chunk, source in rows:
            # Dismissed chunks are hidden until next explicit re-scan (scan_source or ingestion)
            try:
                stored_cats = json.loads(chunk.pii_categories or "[]")
            except Exception:
                stored_cats = []
            if "_dismissed" in stored_cats:
                continue
            matches = scan_text(chunk.content, disabled=disabled, custom_words=custom_words)
            if not matches and not chunk.is_pii:
                continue
            if only_flagged and not (chunk.is_pii or any(m.strong for m in matches)):
                continue
            source_id = None
            source_name = ""
            source_label = "internal"
            if source is not None:
                source_id = source.id
                source_name = source.name
                source_label = source.label
            out.append(
                {
                    "chunk_id": chunk.id,
                    "source_id": source_id,
                    "source_name": source_name,
                    "source_label": source_label,
                    "kind": chunk.kind,
                    "content": chunk.content,
                    "snippet": chunk.content[:300],
                    "is_pii": chunk.is_pii,
                    "categories": serialize_matches(matches),
                }
            )
        return out

    @router.post("/review/{chunk_id}", response_model=schemas.PiiChunkOut)
    def decide(chunk_id: int, payload: schemas.PiiReviewDecision, db: Session = Depends(get_db)) -> dict:
        chunk = db.get(Chunk, chunk_id)
        if chunk is None:
            raise HTTPException(status_code=404, detail="Chunk not found")
        chunk.is_pii = payload.is_pii
        if payload.is_pii:
            # confirming PII - store current matches so it stays flagged even if live scan weakens
            cfg_tmp = load_config(db)
            matches_tmp = scan_text(chunk.content, disabled=cfg_tmp["disabled"], custom_words=cfg_tmp["custom_words"])
            chunk.pii_categories = json.dumps([m.category for m in matches_tmp] or ["_manual"])
        else:
            # NOT PII - dismiss: hide from review until next explicit re-scan
            chunk.pii_categories = json.dumps(["_dismissed"])
        db.commit()
        cfg = load_config(db)
        matches = scan_text(chunk.content, disabled=cfg["disabled"], custom_words=cfg["custom_words"])
        return {
            "chunk_id": chunk.id,
            "source_id": None,
            "source_name": "",
            "source_label": "internal",
            "kind": chunk.kind,
            "content": chunk.content,
            "snippet": chunk.content[:300],
            "is_pii": chunk.is_pii,
            "categories": serialize_matches(matches),
        }

    @router.post("/scan/{source_id}")
    def scan_source(source_id: int, db: Session = Depends(get_db)) -> dict:
        """Re-run the PII scan over a source's chunks (picks up config changes)."""
        from ..models import IngestedItem

        source = db.get(Source, source_id)
        if source is None:
            raise HTTPException(status_code=404, detail="Source not found")
        cfg = load_config(db)
        item_ids = db.execute(
            select(IngestedItem.id).where(IngestedItem.source_id == source_id)
        ).scalars().all()
        chunks = db.execute(
            select(Chunk).where(Chunk.item_id.in_(item_ids))
        ).scalars().all()
        flagged = 0
        for chunk in chunks:
            matches = scan_text(chunk.content, disabled=cfg["disabled"], custom_words=cfg["custom_words"])
            chunk.pii_categories = json.dumps([m.category for m in matches])
            chunk.is_pii = any(m.strong for m in matches)
            if chunk.is_pii:
                flagged += 1
        db.commit()
        logger.info("PII scan of source %d: %d/%d chunks flagged", source_id, flagged, len(chunks))
        return {"source_id": source_id, "chunks": len(chunks), "flagged": flagged}

    return router
