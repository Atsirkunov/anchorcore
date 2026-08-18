"""Distillation (B18) + IDF signal gating (B35).

`distill_and_store` normalizes chat-like windows into searchable Q&A units
(kind='distilled'); `signal` computes the B18 IDF-gated embeddability score.
Extracted from pipeline.py so the ingestion orchestration stays thin.
"""

import asyncio
import json
import logging
import math
import re
from collections import Counter

from sqlalchemy import select
from sqlalchemy.orm import Session

from .classifier import DISTILL_DOC_TYPES
from .config import settings
from .hashing import window_hash
from .models import Chunk, IngestedItem

logger = logging.getLogger(__name__)


async def distill_and_store(
    classifier,
    db: Session,
    item: IngestedItem,
    doc,
    windows: list[str],
    force_reclassify: bool,
    cloud_trusted: bool = True,
) -> None:
    """B18: normalize chat-like windows into searchable Q&A units.

    Mirrors the classification hash-skip: unchanged windows don't re-distill
    (cheap on re-sync/reclassify). Distilled units are stored as chunks with
    kind='distilled' and a normalized 'Q: … A: …' content so they embed well.
    Old distilled chunks are dropped first so re-runs don't duplicate."""
    prev_hashes = json.loads(item.distill_hashes or "{}")
    base_ref = doc.source_ref or doc.title
    refs = [f"{base_ref} §{index}" if len(windows) > 1 else base_ref for index in range(1, len(windows) + 1)]
    sem = asyncio.Semaphore(settings.classifier_concurrency)

    async def distill_one(window: str, ref: str, index: int) -> list[dict] | None:
        h = window_hash(window)
        if not force_reclassify and prev_hashes.get(str(index)) == h:
            return None
        async with sem:
            try:
                return await classifier.distill(window, ref, cloud_trusted=cloud_trusted)
            except Exception as exc:  # noqa: BLE001
                logger.warning("distillation failed for window %s (%s)", index, exc)
                return None

    results = await asyncio.gather(
        *(distill_one(w, r, i) for i, (w, r) in enumerate(zip(windows, refs), start=1))
    )

    new_hashes = dict(prev_hashes)
    old_distilled = db.execute(
        select(Chunk).where(Chunk.item_id == item.id, Chunk.kind == "distilled")
    ).scalars().all()
    for chunk in old_distilled:
        db.delete(chunk)
    db.flush()

    max_units = settings.distill_max_units
    total = 0
    for index, (window, batch) in enumerate(zip(windows, results), start=1):
        new_hashes[str(index)] = window_hash(window)
        if batch is None:
            continue
        for unit in batch[:max_units]:
            total += 1
            content = f"Q: {unit['question']}\nA: {unit['answer']}"
            if unit.get("terms"):
                content += f"\nTerms: {', '.join(unit['terms'])}"
            if unit.get("systems"):
                content += f"\nSystems: {', '.join(unit['systems'])}"
            db.add(
                Chunk(
                    item_id=item.id,
                    kind="distilled",
                    source_ref=refs[index - 1],
                    content=content,
                )
            )
    item.distill_hashes = json.dumps(new_hashes)
    db.commit()
    if total:
        logger.info("distilled %d unit(s) from %s", total, doc.title)


def signal(content: str, chunks: list[Chunk]) -> float:
    """B18 IDF-gated signal score: mean inverse document frequency of the
    chunk's tokens across the corpus (here: the item's chunks). Short filler
    and sparse/rare-token content score low and are skipped from embedding.

    B35: scoped to the item's corpus (its chunks) — same semantics as the old
    per-item gate in pipeline.py."""
    doc_freq: Counter[str] = Counter()
    for c in chunks:
        doc_freq.update(re.findall(r"[a-z0-9_]{3,}", c.content.lower()))
    n = max(1, len(chunks))
    tokens = re.findall(r"[a-z0-9_]{3,}", content.lower())
    if not tokens:
        return 0.0
    # idf = log(n / df); rare tokens (df small) → higher idf. We want the
    # *signal* = how distinctive this chunk's vocabulary is. Common filler
    # words shared across chunks contribute ~0 idf.
    total = 0.0
    for t in set(tokens):
        df = doc_freq.get(t, 0)
        if df == 0:
            continue
        total += max(0.0, math.log((n + 1) / (df + 1)))
    return total / len(set(tokens))


__all__ = ["distill_and_store", "signal", "DISTILL_DOC_TYPES"]
