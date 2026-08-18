"""B12: chunk cleaning, heading-aware chunking, FTS5 hybrid retrieval.
B33: vec0 index vector retrieval (index availability, correctness, perf)."""

import time
from datetime import datetime, timedelta, timezone

from app.answer_engine import _age_decay, _rrf_fuse
from app.cleaning import clean_text, repeated_lines, strip_repeated
from app.config import settings
from app.pipeline import chunk_document

from tests.test_smoke import start_and_wait


def _vec0_available() -> bool:
    import sqlite3

    try:
        import sqlite_vec  # noqa: F401
    except ImportError:
        return False
    conn = sqlite3.connect(":memory:")
    try:
        conn.enable_load_extension(True)
    except AttributeError:
        return False
    try:
        sqlite_vec.load(conn)
    except Exception:  # noqa: BLE001
        return False
    conn.close()
    return True


def _pack_vector(vector: list[float]) -> bytes:
    import struct

    return b"".join(struct.pack("<f", v) for v in vector)


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


def test_diversity_cap_limits_per_item(client, tmp_path):
    """With multiple files, one file must not monopolize the top results."""
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
    from app.app_settings import SettingsService
    from app.embedder import Embedder
    from app.db import SessionLocal
    from app.secrets import SecretStore

    engine = AnswerEngine(
        Embedder(SettingsService(settings, SecretStore(settings.data_dir / "secrets.enc"))),
        SettingsService(settings, SecretStore(settings.data_dir / "secrets.enc")),
    )
    with SessionLocal() as db:
        keyword_hits = engine._keyword_search(db, "DVCA")
        assert keyword_hits, "expected keyword hits"
        cap = settings.retrieval_max_per_source
        assert cap >= 1
        fused = engine._fuse_and_rank(db, None, keyword_hits)
        assert fused, "expected fused hits"
        # 4 identical files → ≥3 distinct items → cap applies per item
        from collections import Counter

        counts = Counter(h["chunk"].item_id for h in fused)
        assert max(counts.values()) <= cap, counts


def test_diversity_cap_relaxed_for_single_file(client, tmp_path):
    """A single big document must be allowed to surface multiple sections
    (no other files to diversify against — the cap must not starve it)."""
    from app.config import settings

    paragraphs = "\n\n".join(
        ["§1 DVCA movement rules."]
        + [
            f"Paragraph number {i} discussing the DVCA movement rules in considerable detail "
            f"so that the section splits into multiple chunks."
            for i in range(1, 60)
        ]
    )
    (tmp_path / "big.md").write_text(paragraphs, encoding="utf-8")
    source = client.post(
        "/sources",
        json={"connector": "folder", "name": "single", "config": {"path": str(tmp_path)}},
    ).json()
    start_and_wait(client, source["id"])

    from app.answer_engine import AnswerEngine
    from app.app_settings import SettingsService
    from app.embedder import Embedder
    from app.db import SessionLocal
    from app.secrets import SecretStore

    engine = AnswerEngine(
        Embedder(SettingsService(settings, SecretStore(settings.data_dir / "secrets.enc"))),
        SettingsService(settings, SecretStore(settings.data_dir / "secrets.enc")),
    )
    with SessionLocal() as db:
        keyword_hits = engine._keyword_search(db, "DVCA")
        from app.models import IngestedItem
        from sqlalchemy import select

        item_id = db.execute(
            select(IngestedItem.id).where(IngestedItem.source_id == source["id"])
        ).scalar_one()
        mine = [h for h in keyword_hits if h["chunk"].item_id == item_id]
        assert len(mine) >= 5, "expected many DVCA chunks from one file"
        fused = engine._fuse_and_rank(db, None, mine)
        assert len(fused) > settings.retrieval_max_per_source, (
            "single-file corpus must not be capped below top_k"
        )


def test_vec_chunks_table_exists(client):
    """B33 DoD: the vec0 virtual table + triggers exist after migration (when
    the sqlite-vec extension is loadable)."""
    if not _vec0_available():
        import pytest

        pytest.skip("sqlite-vec not loadable in this Python")
    import sqlite3

    from app.db import engine

    with engine.connect() as conn:
        dbapi = conn.connection.driver_connection
        table = dbapi.execute(
            "SELECT name FROM sqlite_master WHERE type='table' AND name='vec_chunks'"
        ).fetchone()
        triggers = [
            r[0]
            for r in dbapi.execute(
                "SELECT name FROM sqlite_master WHERE type='trigger' AND name LIKE 'vec_chunks_%'"
            ).fetchall()
        ]
    assert table is not None, "vec_chunks table must exist after migration"
    assert {"vec_chunks_ai", "vec_chunks_ad", "vec_chunks_au"} <= set(triggers), triggers


def test_vec0_insert_and_retrieval(client, tmp_path):
    """B33: embeddings written through the trigger are retrievable by vec0 with
    cosine scoring; a wrong-dimension blob is ignored rather than breaking the
    chunk write."""
    if not _vec0_available():
        import pytest

        pytest.skip("sqlite-vec not loadable in this Python")
    from app.db import SessionLocal
    from app.models import Chunk
    from sqlalchemy import delete, select

    dim = settings.embed_dim
    # two vectors: one near the query, one far
    near = [0.1] * dim
    far = [-1.0] * dim
    query = [0.1] * dim

    with SessionLocal() as db:
        db.execute(delete(Chunk).where(Chunk.content.in_(["vec-near", "vec-far", "vec-wrong"])))
        db.flush()
        for content, vec in [("vec-near", near), ("vec-far", far)]:
            db.execute(
                Chunk.__table__.insert().values(
                    kind="document",
                    content=content,
                    source_ref="vec-test",
                    embedding=_pack_vector(vec),
                )
            )
        db.execute(
            Chunk.__table__.insert().values(
                kind="document",
                content="vec-wrong",
                source_ref="vec-test",
                embedding=_pack_vector([0.1, 0.2, 0.3, 0.4]),
            )
        )
        db.commit()

        from app.answer_engine import AnswerEngine
        from app.app_settings import SettingsService
        from app.embedder import Embedder
        from app.secrets import SecretStore

        engine = AnswerEngine(
            Embedder(SettingsService(settings, SecretStore(settings.data_dir / "secrets.enc"))),
            SettingsService(settings, SecretStore(settings.data_dir / "secrets.enc")),
        )
        hits = engine._vector_search(db, query)
        by_content = {h["chunk"].content: h["score"] for h in hits}
        assert "vec-near" in by_content, "near vector must be retrieved via vec0"
        assert "vec-far" not in by_content, "far vector must not pass the 0.2 cosine cutoff"
        assert "vec-wrong" not in by_content, "wrong-dim blob must not be indexed"
        assert by_content["vec-near"] > 0.99, by_content

        # cleanup so shared-DB tests stay isolated
        db.execute(delete(Chunk).where(Chunk.content.in_(["vec-near", "vec-far", "vec-wrong"])))
        db.commit()


def test_vec0_project_scoping(client, tmp_path):
    """B33: vec0 retrieval is scoped to a project's sources (B15)."""
    if not _vec0_available():
        import pytest

        pytest.skip("sqlite-vec not loadable in this Python")
    from app.db import SessionLocal
    from app.models import Chunk, IngestedItem
    from sqlalchemy import delete, select

    dim = settings.embed_dim
    with SessionLocal() as db:
        rows = db.execute(
            select(IngestedItem.id, IngestedItem.source_id)
        ).all()
        if not rows:
            pytest.skip("no ingested items available in shared test DB")

    from app.answer_engine import AnswerEngine
    from app.app_settings import SettingsService
    from app.embedder import Embedder
    from app.secrets import SecretStore

    engine = AnswerEngine(
        Embedder(SettingsService(settings, SecretStore(settings.data_dir / "secrets.enc"))),
        SettingsService(settings, SecretStore(settings.data_dir / "secrets.enc")),
    )
    query = [0.1] * dim
    # scope to a source that has no chunks matching near the query → empty set
    with SessionLocal() as db:
        # build an itemless chunk first so scoping has something to exclude
        db.execute(delete(Chunk).where(Chunk.content == "vec-scope"))
        db.flush()
        db.execute(
            Chunk.__table__.insert().values(
                kind="document",
                content="vec-scope",
                source_ref="vec-test",
                embedding=_pack_vector([0.1] * dim),
            )
        )
        db.commit()
        # an empty project scope must yield no hits (nothing to exclude / include)
        hits = engine._vector_search(db, query, source_ids=set())
        assert hits == [], "empty project scope must return no vector hits"
        db.execute(delete(Chunk).where(Chunk.content == "vec-scope"))
        db.commit()


def test_vec0_perf_probe():
    """B33 DoD: 1k-chunk vec0 query <100ms (vs the O(N) Python scan)."""
    import sqlite3

    if not _vec0_available():
        import pytest

        pytest.skip("sqlite-vec not loadable in this Python")

    conn = sqlite3.connect(":memory:")
    conn.enable_load_extension(True)
    import sqlite_vec

    sqlite_vec.load(conn)
    dim = 768
    conn.execute(f"CREATE VIRTUAL TABLE v USING vec0(embedding float[{dim}] distance_metric=cosine)")
    import random

    rng = random.Random(42)
    start = time.perf_counter()
    for i in range(1000):
        vec = [rng.uniform(-1, 1) for _ in range(dim)]
        conn.execute("INSERT INTO v(rowid, embedding) VALUES (?, ?)", (i + 1, _pack_vector(vec)))
    insert_ms = (time.perf_counter() - start) * 1000

    query = [rng.uniform(-1, 1) for _ in range(dim)]
    import json

    start = time.perf_counter()
    rows = conn.execute(
        "SELECT rowid, distance FROM v WHERE embedding MATCH :q AND k = 8",
        {"q": json.dumps(query)},
    ).fetchall()
    latency_ms = (time.perf_counter() - start) * 1000
    conn.close()

    assert len(rows) == 8, rows
    assert latency_ms < 100, f"vec0 query over 1k chunks took {latency_ms:.1f}ms (insert {insert_ms:.1f}ms)"

