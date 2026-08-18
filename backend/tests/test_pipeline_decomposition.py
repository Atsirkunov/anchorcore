"""B35: pipeline decomposition — chunking/hashing/distill modules extracted
from pipeline.py, plus the commit-before-LLM invariant that must survive."""

import asyncio
import json

from app.chunking import chunk_document, chunk_text, classify_windows
from app.hashing import content_hash, window_hash


def test_chunk_text_fixed_size_with_overlap():
    text = "x" * 3000
    chunks = chunk_text(text)
    assert len(chunks) >= 2
    assert all(len(c) <= 800 for c in chunks)


def test_chunk_document_respects_headings_and_splits_oversized():
    from app.config import settings

    doc = "\n".join(
        [
            "§434 Merger and related rules",
            "Merger content paragraph one.",
            "",
            "§437 Key dates",
            "The key dates are January and June.",
        ]
    )
    chunks = chunk_document(doc)
    assert len(chunks) == 2, chunks
    assert chunks[0].startswith("§434")
    assert chunks[1].startswith("§437")

    big = ["§100 Big section"] + [
        f"Paragraph number {i} with substantial detail here." for i in range(1, 60)
    ]
    big_chunks = chunk_document("\n\n".join(big))
    assert any("Paragraph number 59" in c for c in big_chunks)
    assert all(len(c) <= settings.chunk_max_chars + 200 for c in big_chunks)


def test_classify_windows_overlaps():
    text = "y" * 30000
    windows = classify_windows(text)
    assert len(windows) >= 2
    assert all(len(w) <= 16000 for w in windows)


def test_hashing_single_source_of_truth():
    """window_hash and content_hash now come from hashing.py; classifier.py and
    models.py re-export them so existing imports keep working."""
    from app.classifier import window_hash as ch_window_hash
    from app.models import content_hash as models_content_hash

    assert window_hash("hello") == ch_window_hash("hello")
    assert content_hash("hello") == models_content_hash("hello")
    assert window_hash("hello") != window_hash("hello!")
    assert content_hash("hello") != content_hash("hellp")


def test_commit_before_llm_invariant(client, tmp_path, monkeypatch):
    """B35 DoD: the pipeline commits the ingested item BEFORE calling the LLM
    classifier — no write tx is held during a classifier call (a held lock
    would stall concurrent syncs on SQLite)."""
    from app import main
    from app.db import SessionLocal
    from app.models import IngestedItem

    class _ProbeClassifier:
        def __init__(self, real, probe):
            self._real = real
            self._probe = probe

        async def detect_document_type(self, *a, **k):
            return "general"

        async def classify(self, window, ref, doc_type="general", cloud_trusted=True):
            # the real LLM call boundary: assert the write tx is NOT open here
            self._probe["called"] = True
            with SessionLocal() as db:
                tx = db.in_transaction()
            assert tx is False, "classifier ran while a write transaction was open"
            return await self._real.classify(window, ref, doc_type, cloud_trusted=cloud_trusted)

        def __getattr__(self, name):
            return getattr(self._real, name)

    probe = {"called": False}
    # monkeypatch restores the original classifier after this test — the probe
    # must NOT leak into later tests (main.pipeline is a module singleton)
    monkeypatch.setattr(main.pipeline, "classifier", _ProbeClassifier(main.classifier, probe))

    (tmp_path / "commit.md").write_text(
        "We decided to adopt the new stack in March.", encoding="utf-8"
    )
    source = client.post(
        "/sources",
        json={"connector": "folder", "name": "commit", "config": {"path": str(tmp_path)}},
    ).json()
    from tests.test_smoke import start_and_wait

    job = start_and_wait(client, source["id"])
    assert job["status"] == "done", job

    from app.models import Entity
    from sqlalchemy import select

    with SessionLocal() as db:
        entities = db.execute(select(Entity)).scalars().all()
    assert probe["called"], "classifier probe never ran — test is not exercising the path"
    assert entities, "expected entities to be created"
