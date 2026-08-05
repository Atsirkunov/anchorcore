"""B12: chunk cleaning, heading-aware chunking, FTS5 hybrid retrieval."""

import time

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
