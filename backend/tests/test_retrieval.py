"""B12: chunk cleaning, heading-aware chunking, FTS5 hybrid retrieval."""

import time
from datetime import datetime, timedelta, timezone

from app.answer_engine import _age_decay, _rrf_fuse
from app.cleaning import clean_text, repeated_lines, strip_repeated
from app.pipeline import chunk_document

from tests.test_smoke import start_and_wait


def test_clean_text_removes_page_numbers_and_control_chars():
    dirty = "Intro\u00ad\x07 text\r\nPage 3 of 12\r\n- 42 -\r\nActual content here.\r\n\r\n\nMore.\n"
    cleaned = clean_text(dirty)
    assert "Page 3 of 12" not in cleaned
    assert "- 42 -" not in cleaned
    assert "\x07" not in cleaned
    assert "\u00ad" not in cleaned
    assert "Intro text" in cleaned
    assert "Actual content here." in cleaned
    assert "More." in cleaned


def test_repeated_lines_detected_and_stripped():
    header = "CONFIDENTIAL — INTERNAL USE ONLY"
    doc = "\n".join(
        [header] * 8 + ["Some actual content line one.", "Another real fact here."] + [header] * 5
    )
    repeated = repeated_lines(doc)
    assert header.lower() in repeated
    assert "some actual content line one." not in repeated
    stripped = strip_repeated(doc, repeated)
    assert header not in stripped
    assert "Some actual content line one." in stripped


def test_chunk_document_respects_headings():
    doc = "\n".join(
        [
            "§434 Merger and related rules",
            "Merger content paragraph one.",
            "",
            "Merger content paragraph two.",
            "",
            "§437 Key dates",
            "The key dates are January and June.",
        ]
    )
    chunks = chunk_document(doc)
    assert len(chunks) == 2, chunks
    assert chunks[0].startswith("§434")
    assert chunks[1].startswith("§437")
    assert "January and June" in chunks[1]


def test_chunk_document_splits_oversized_sections():
    from app.config import settings

    big = ["§100 Big section"] + [f"Paragraph number {i} with substantial detail here." for i in range(1, 60)]
    doc = "\n\n".join(big)
    chunks = chunk_document(doc)
    assert len(chunks) >= 2
    for chunk in chunks:
        assert len(chunk) <= settings.chunk_max_chars + 200, len(chunk)
    assert any("Paragraph number 59" in c for c in chunks)


def test_hybrid_keyword_retrieval(client, tmp_path):
    """Acronyms must hit directly via FTS even without embeddings (CI has no
    Ollama, so vector search is unavailable — this exercises the keyword half
    of the hybrid path end-to-end)."""
    (tmp_path / "rules.md").write_text(
        "§5 DVCA movement rules\n"
        "The DVCA movement must be reported with the DVSE identifier.\n\n"
        "§9 Other stuff\n"
        "Unrelated paragraph about parking regulations.\n",
        encoding="utf-8",
    )
    source = client.post(
        "/sources",
        json={"connector": "folder", "name": "b12", "config": {"path": str(tmp_path)}},
    ).json()
    job = start_and_wait(client, source["id"])
    assert job["status"] == "done"

    resp = client.post("/qa", json={"question": "What are the DVCA movement rules?"})
    assert resp.status_code == 200, resp.text
    citations = resp.json()["citations"]
    assert citations, "expected cited chunks"
    assert any("DVCA" in c["snippet"] or "DVCA" in c["source_ref"] for c in citations), citations


def test_fts_table_exists(client):
    import sqlite3

    from app.db import engine

    with engine.connect() as conn:
        dbapi = conn.connection.driver_connection
        row = dbapi.execute("SELECT name FROM sqlite_master WHERE type='table' AND name='chunks_fts'").fetchone()
    assert row is not None, "chunks_fts table must exist after migration"


class _FakeChunk:
    def __init__(self, cid):
        self.id = cid
        self.created_at = datetime.now(timezone.utc)


def _hit(cid, source_id=1, created_at=None):
    chunk = _FakeChunk(cid)
    if created_at is not None:
        chunk.created_at = created_at
    return {"chunk": chunk, "entity": None, "item": None, "source_id": source_id, "score": 0.0}


def test_rrf_consensus_beats_single_vote():
    """A chunk ranked in both lists beats one ranked #1 in a single list."""
    vec = [_hit(1), _hit(2)]
    kw = [_hit(2)]
    fused = _rrf_fuse(vec, kw, keyword_weight=1.0)
    by_id = {h["chunk"].id: h["score"] for h in fused}
    # chunk 2: #2 in vec + #1 in kw = 1/62 + 1/61 (consensus)
    # chunk 1: #1 in vec only = 1/61 (single strong vote)
    assert by_id[2] > by_id[1], by_id
    assert abs(by_id[2] - (1 / 62 + 1 / 61)) < 1e-9
    assert abs(by_id[1] - 1 / 61) < 1e-9


def test_rrf_keyword_weight_scales_list():
    vec = [_hit(1), _hit(2)]
    kw = [_hit(2), _hit(1)]
    fused_zero = _rrf_fuse(vec, kw, keyword_weight=0.0)
    by_id = {h["chunk"].id: h["score"] for h in fused_zero}
    assert abs(by_id[1] - 1 / 61) < 1e-9 and abs(by_id[2] - 1 / 62) < 1e-9


def test_age_decay_favors_recent():
    now = datetime.now(timezone.utc)
    fresh = _age_decay(now - timedelta(days=10), halflife_days=365)
    old = _age_decay(now - timedelta(days=365), halflife_days=365)
    assert fresh > old
    assert abs(old - 0.5) < 1e-6  # one halflife → half score
    assert abs(_age_decay(None, 365) - 1.0) < 1e-9


def test_diversity_cap_limits_per_source(client, tmp_path):
    """One source must not monopolize the top results (per-source cap)."""
    from app.config import settings

    (tmp_path / "a.md").write_text("§1 DVCA movement rules.\n", encoding="utf-8")
    (tmp_path / "b.md").write_text("§1 DVCA movement rules.\n", encoding="utf-8")
    (tmp_path / "c.md").write_text("§1 DVCA movement rules.\n", encoding="utf-8")
    (tmp_path / "d.md").write_text("§1 DVCA movement rules.\n", encoding="utf-8")
    source = client.post(
        "/sources",
        json={"connector": "folder", "name": "cap", "config": {"path": str(tmp_path)}},
    ).json()
    start_and_wait(client, source["id"])

    from app.answer_engine import AnswerEngine
    from app.embedder import Embedder
    from app.db import SessionLocal

    engine = AnswerEngine(Embedder())
    with SessionLocal() as db:
        keyword_hits = engine._keyword_search(db, "DVCA")
        assert keyword_hits, "expected keyword hits"
        cap = settings.retrieval_max_per_source
        assert cap >= 1
        # same-source hits must be capped; other sources may contribute too
        fused = engine._fuse_and_rank(db, None, keyword_hits)
        assert fused, "expected fused hits"
        from collections import Counter

        counts = Counter(h["source_id"] for h in fused)
        assert max(counts.values()) <= cap, counts
        this_source = next(h for h in fused if h["source_id"] == source["id"])
        assert this_source is not None
