"""B38: testing & observability uplift — paginated duplicate scan, isolated DB
fixture, and the retrieval latency probe (the vec0 probe itself lives in
test_retrieval.py alongside the B33 tests)."""

import time

from app.answer_engine import _rrf_fuse


def test_duplicates_paginates_and_filters(client, tmp_path):
    """B38 DoD: GET /review/duplicates accepts limit/offset/kind, returns at
    most `limit` proposals, and offset pages skip already-returned ones."""
    # insert near-identical entities directly under a real source (the
    # shared-DB corpus is not guaranteed to produce duplicates under the
    # rule-based classifier; Entity.item_id + IngestedItem.source_id are NOT NULL)
    (tmp_path / "seed.md").write_text("seed", encoding="utf-8")
    source = client.post(
        "/sources",
        json={"connector": "folder", "name": "dups", "config": {"path": str(tmp_path)}},
    ).json()

    from app.db import SessionLocal
    from app.models import Entity, IngestedItem

    summaries = [
        "Annual budget approved for the fiscal year by the board",
        "Fiscal year annual budget approved by the board",
        "Hiring freeze stays in place until the third quarter",
        "The hiring freeze remains until Q3 of this year",
        "New Berlin office opens in January for the sales team",
        "The Berlin office opens in January for sales",
    ]
    with SessionLocal() as db:
        items = []
        for s in summaries:
            item = IngestedItem(
                source_id=source["id"],
                external_id=f"dup-{abs(hash(s))}",
                title=s[:60],
                text=s,
                content_hash=s,
                author="fixture",
                stale=False,
            )
            db.add(item)
            db.flush()
            items.append(item)
        for item, s in zip(items, summaries):
            db.add(
                Entity(
                    item_id=item.id,
                    kind="decision",
                    summary=s,
                    reasoning="test fixture",
                    confidence=0.8,
                    author="fixture",
                    source_ref="dups-fixture",
                    status="unverified",
                )
            )
        db.commit()

    all_proposals = client.get("/review/duplicates").json()
    assert isinstance(all_proposals, list)
    assert len(all_proposals) >= 1, "expected duplicate proposals for near-identical entities"

    limited = client.get("/review/duplicates", params={"limit": 1}).json()
    assert len(limited) <= 1, limited

    # offset=0 and offset=1 should not return the same first proposal id
    page0 = client.get("/review/duplicates", params={"limit": 1, "offset": 0}).json()
    page1 = client.get("/review/duplicates", params={"limit": 1, "offset": 1}).json()
    if page0 and page1:
        assert page0[0]["id"] != page1[0]["id"], "offset must skip the previous page"

    # kind filter is accepted and returns a well-shaped list (the first scan
    # already recorded proposals, so subsequent calls may return fewer)
    by_kind = client.get("/review/duplicates", params={"kind": "decision"}).json()
    assert isinstance(by_kind, list)
    assert all("entity_a_id" in p and "entity_b_id" in p for p in by_kind)

    # limit is clamped (never a giant page)
    huge = client.get("/review/duplicates", params={"limit": 9999}).json()
    assert len(huge) <= 100

    # cleanup so the shared test DB stays predictable
    from sqlalchemy import delete

    with SessionLocal() as db:
        db.execute(delete(Entity).where(Entity.source_ref == "dups-fixture"))
        db.execute(delete(IngestedItem).where(IngestedItem.external_id.like("dup-%")))
        db.commit()


def test_isolated_db_fixture_is_empty(isolated_db):
    """B38 DoD: a test can request an empty DB and assert global emptiness
    (the shared test DB accumulates, so this needs the isolated fixture)."""
    engine, session = isolated_db
    from app.models import Source
    from sqlalchemy import func, select

    count = session.execute(select(func.count()).select_from(Source)).scalar_one()
    assert count == 0, "isolated DB must start empty"
    session.close()


def test_retrieval_latency_probe_smoke():
    """B38 DoD: the retrieval perf probe plumbing is callable and fast enough
    on a small synthetic corpus (the real 1k-chunk vec0 threshold is in
    test_retrieval.py::test_vec0_perf_probe)."""
    import json
    import sqlite3

    try:
        import sqlite_vec  # noqa: F401
    except ImportError:
        import pytest

        pytest.skip("sqlite-vec not installed")

    conn = sqlite3.connect(":memory:")
    conn.enable_load_extension(True)
    sqlite_vec.load(conn)
    dim = 768
    conn.execute(f"CREATE VIRTUAL TABLE v USING vec0(embedding float[{dim}] distance_metric=cosine)")

    import random
    import struct

    rng = random.Random(7)
    blobs = [
        (i + 1, b"".join(struct.pack("<f", rng.uniform(-1, 1)) for _ in range(dim)))
        for i in range(50)
    ]
    conn.executemany("INSERT INTO v(rowid, embedding) VALUES (?, ?)", blobs)
    query = [0.0] * dim
    start = time.perf_counter()
    rows = conn.execute(
        "SELECT rowid, distance FROM v WHERE embedding MATCH :q AND k = 8",
        {"q": json.dumps(query)},
    ).fetchall()
    latency_ms = (time.perf_counter() - start) * 1000
    conn.close()
    assert len(rows) == 8
    assert latency_ms < 100, f"probe too slow: {latency_ms:.1f}ms"
