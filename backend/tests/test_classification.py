"""B26: document-aware classification, reviewable window context, cheaper reclassify."""

import asyncio
import json

from app.classifier import PROMPTS, Classifier, window_hash
from app.cleaning import clean_text
from app.pipeline import classify_windows


def _make_svc():
    from app.app_settings import SettingsService
    from app.config import settings
    from app.secrets import SecretStore

    return SettingsService(settings, SecretStore(settings.data_dir / "secrets.enc"))


def test_standards_prompt_suppresses_decision_action():
    prompt = PROMPTS["standards"].lower()
    assert "do not extract" in prompt
    assert "decision" in prompt and "action" in prompt  # explicitly forbidden
    # the extraction kind list only allows note
    assert '"kind": "note"' in PROMPTS["standards"]


def test_meeting_prompt_allows_all_kinds():
    prompt = PROMPTS["meeting"]
    assert "decision: a choice made in the meeting" in prompt
    assert "action: something someone agreed to do" in prompt


def test_window_hash_deterministic():
    assert window_hash("hello world") == window_hash("hello world")
    assert window_hash("hello world") != window_hash("hello world!")


def test_classify_windows_sizes():
    text = "x" * 40000
    windows = classify_windows(text)
    assert len(windows) >= 2


def test_detect_document_type_llm_payload(client):
    """detect_document_type must hit the classifier endpoint with the
    type-detection prompt and return one of the known types."""
    from app.config import settings

    svc = _make_svc()
    svc.set("classifier_base_url", "https://api.openai.com/v1")
    svc.set("classifier_api_key", "sk-detect-test")

    captured = {}

    class _FakeClient:
        def __init__(self, *a, **kw):
            pass

        async def __aenter__(self):
            return self

        async def __aexit__(self, *a):
            return False

        async def post(self, url, headers=None, json=None):
            captured["json"] = json

            class _Resp:
                def raise_for_status(self):
                    pass

                def json(self):
                    return {"choices": [{"message": {"content": "standards"}}]}

            return _Resp()

    import app.classifier as mod

    original = mod.RetryClient
    mod.RetryClient = _FakeClient
    try:
        result = asyncio.run(Classifier(svc).detect_document_type("some regulatory text"))
    finally:
        mod.RetryClient = original

    assert result == "standards"
    sys_prompt = captured["json"]["messages"][0]["content"]
    assert "what kind of document" in sys_prompt.lower()
    svc.clear("classifier_base_url")
    svc.clear("classifier_api_key")


def test_classify_passes_doc_type_prompt(client):
    """classify must select the per-type prompt (e.g. standards → no
    decision/action in the system message)."""
    from app.config import settings

    svc = _make_svc()
    svc.set("classifier_base_url", "https://api.openai.com/v1")
    svc.set("classifier_api_key", "sk-type-test")

    captured = {}

    class _FakeClient:
        def __init__(self, *a, **kw):
            pass

        async def __aenter__(self):
            return self

        async def __aexit__(self, *a):
            return False

        async def post(self, url, headers=None, json=None):
            captured["json"] = json

            class _Resp:
                def raise_for_status(self):
                    pass

                def json(self):
                    return {"choices": [{"message": {"content": '{"items": []}'}}]}

            return _Resp()

    import app.classifier as mod

    original = mod.RetryClient
    mod.RetryClient = _FakeClient
    try:
        asyncio.run(Classifier(svc).classify("rule text", "ref", doc_type="standards"))
    finally:
        mod.RetryClient = original

    sys_prompt = captured["json"]["messages"][0]["content"]
    assert "do not extract" in sys_prompt.lower()
    svc.clear("classifier_base_url")
    svc.clear("classifier_api_key")


def test_entity_window_context_attached(client, tmp_path):
    """Entities created from a standards doc carry window_text + index, and
    reclassify skips unchanged windows (no new entities on second run)."""
    (tmp_path / "rules.md").write_text(
        "We decided that the account servicer must send a preliminary advice message.\n"
        "Status: the record date must be included for all mandatory events.\n",
        encoding="utf-8",
    )
    source = client.post(
        "/sources",
        json={"connector": "folder", "name": "b26", "config": {"path": str(tmp_path)}},
    ).json()
    from tests.test_smoke import start_and_wait

    job = start_and_wait(client, source["id"])
    assert job["status"] == "done"

    from app.db import SessionLocal
    from app.models import Entity, IngestedItem
    from sqlalchemy import select

    with SessionLocal() as db:
        item = db.execute(select(IngestedItem).where(IngestedItem.source_id == source["id"])).scalar_one()
        entities = db.execute(select(Entity).where(Entity.item_id == item.id)).scalars().all()
        assert entities, "expected entities"
        # window_text must be populated with the source excerpt the classifier saw
        assert all(e.window_text for e in entities), "window_text must be set"
        assert any(e.window_index is not None for e in entities), "window_index must be set"
        first_hashes = item.window_hashes
        assert first_hashes != "{}", "window hashes must be persisted"

        # reclassify: unchanged windows are skipped → no new entities
        job2 = start_and_wait(client, source["id"], kind="reclassify")
        assert job2["status"] == "done"
        entities2 = db.execute(select(Entity).where(Entity.item_id == item.id)).scalars().all()
        assert len(entities2) == len(entities), "unchanged windows must not re-classify"

    # review endpoint exposes window_text
    low = client.get("/review/low-confidence").json()
    assert all("window_text" in e for e in low), "review must expose window_text"


def test_windows_larger_for_cloud():
    """16k-char windows → far fewer classifier calls on big docs."""
    from app.config import settings

    assert settings.classify_window_chars >= 12000
    text = "paragraph. " * 8000  # ~100k chars
    windows = classify_windows(text)
    expected = max(1, (len(text) - settings.classify_window_chars) // (settings.classify_window_chars // 2) + 1)
    assert len(windows) <= expected + 1
