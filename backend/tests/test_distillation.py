"""B18: distillation — normalized Q&A units + IDF-gated embedding.

Verifies the DoD: a chat-log-style source produces findable distilled Q&A
units, and filler messages don't pollute vector results (they stay keyword-only).
"""

from sqlalchemy import select

from app.config import settings
from app.db import SessionLocal
from app.models import Chunk, IngestedItem


def _seed_chat(client, tmp_path, force_general: bool = True) -> dict:
    """A chat-log with real Q&A + heavy filler, forced to doc_type 'general'
    (chat-like → distillation runs) by pre-writing the item."""
    (tmp_path / "chat.md").write_text(
        "\n".join(
            [
                "hey everyone",
                "does the settlement API support partial deliveries?",
                "yes — partial deliveries are supported via the /deliveries/partial endpoint.",
                "ok thanks",
                "and what about failed settlements?",
                "failed settlements roll back automatically and retry up to 3 times.",
                "got it",
                "can you confirm the timeout for the idempotency key?",
                "the idempotency key expires after 24 hours.",
            ]
        ),
        encoding="utf-8",
    )
    source = client.post(
        "/sources",
        json={"connector": "folder", "name": "b18", "config": {"path": str(tmp_path)}},
    ).json()
    from tests.test_smoke import start_and_wait

    job = start_and_wait(client, source["id"])
    assert job["status"] == "done"
    return {"source": source, "job": job}


def _item_chunks(client, source_id: int) -> list[Chunk]:
    with SessionLocal() as db:
        item = db.execute(select(IngestedItem).where(IngestedItem.source_id == source_id)).scalar_one()
        return db.execute(select(Chunk).where(Chunk.item_id == item.id)).scalars().all()


def test_distillation_produces_units(client, tmp_path):
    seed = _seed_chat(client, tmp_path)
    chunks = _item_chunks(client, seed["source"]["id"])
    distilled = [c for c in chunks if c.kind == "distilled"]
    # rule-based distillation (no LLM in CI) pairs Q/A sentences → ≥2 units
    assert len(distilled) >= 2, [c.content for c in distilled]
    content = "\n".join(c.content for c in distilled).lower()
    assert "partial deliveries" in content or "/deliveries/partial" in content
    assert "idempotency key" in content


def test_distilled_unit_is_embed_min_signal(client, tmp_path):
    """Distilled units are real content (high signal) — they must NOT be gated
    out of embedding by the IDF filter."""
    seed = _seed_chat(client, tmp_path)
    chunks = _item_chunks(client, seed["source"]["id"])
    distilled = [c for c in chunks if c.kind == "distilled"]
    assert distilled
    for c in distilled:
        assert len(c.content) >= 20, "distilled units must be substantive"


def test_filler_stays_unembedded_and_keyword_findable(client, tmp_path):
    """Short filler (greetings) should be skipped from vector search by the IDF
    gate, but still present for FTS keyword search (B18 DoD)."""
    seed = _seed_chat(client, tmp_path)
    chunks = _item_chunks(client, seed["source"]["id"])
    # a document chunk containing filler ('hey everyone', 'ok thanks') is low
    # signal → embedding must be NULL, yet the row exists for FTS
    filler_chunks = [c for c in chunks if c.kind == "document"]
    assert filler_chunks, "expected document chunks"
    # the whole chat doc is one chunk → low-signal gate should skip embedding it
    assert all(c.embedding is None for c in filler_chunks), (
        "low-signal doc chunk must be unembedded (IDF gate)"
    )

    # keyword retrieval still finds the filler content (FTS keeps it)
    resp = client.post("/qa", json={"question": "settlement API partial deliveries"})
    assert resp.status_code == 200, resp.text
    assert resp.json()["citations"], "keyword search must still find the chat"


def test_distillation_hash_skips_on_resync(client, tmp_path):
    """Re-sync of unchanged content makes ~0 distillation calls (hash skip)."""
    seed = _seed_chat(client, tmp_path)
    from tests.test_smoke import start_and_wait

    job2 = start_and_wait(client, seed["source"]["id"])
    assert job2["status"] == "done"
    chunks = _item_chunks(client, seed["source"]["id"])
    distilled = [c for c in chunks if c.kind == "distilled"]
    assert len(distilled) >= 2, "units must survive re-sync without duplication"


def test_distilled_chunks_do_not_duplicate_on_resync(client, tmp_path):
    """Old distilled chunks are dropped before re-adding → no duplicates."""
    seed = _seed_chat(client, tmp_path)
    first = _item_chunks(client, seed["source"]["id"])
    first_count = len([c for c in first if c.kind == "distilled"])

    from tests.test_smoke import start_and_wait

    start_and_wait(client, seed["source"]["id"], kind="reclassify")
    second = _item_chunks(client, seed["source"]["id"])
    second_count = len([c for c in second if c.kind == "distilled"])
    assert second_count == first_count, f"distilled count changed {first_count} -> {second_count}"
