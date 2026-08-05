"""Follow-up questions: rewrite + conversation context in generation."""

import asyncio


def _make_svc():
    from app.app_settings import SettingsService
    from app.config import settings
    from app.secrets import SecretStore

    return SettingsService(settings, SecretStore(settings.data_dir / "secrets.enc"))


def test_ask_accepts_history_payload(client, tmp_path):
    """POST /qa with history must return 200 (schema accepts turns)."""
    (tmp_path / "f.md").write_text("§1 Merger details: MRGR is an exchange of securities.\n", encoding="utf-8")
    source = client.post(
        "/sources",
        json={"connector": "folder", "name": "fup", "config": {"path": str(tmp_path)}},
    ).json()
    from tests.test_smoke import start_and_wait

    start_and_wait(client, source["id"])

    resp = client.post(
        "/qa",
        json={
            "question": "show the movements for it",
            "history": [
                {"role": "user", "content": "merger details"},
                {"role": "assistant", "content": "MRGR is a merger event."},
            ],
        },
    )
    assert resp.status_code == 200, resp.text
    assert resp.json()["answer"]


def test_rewrite_used_for_retrieval():
    """A follow-up must be rewritten to a standalone query before retrieval,
    and the conversation must be present in the generation payload."""
    from app.answer_engine import AnswerEngine
    from app.embedder import Embedder

    svc = _make_svc()
    svc.set("answer_reasoning_effort", "none")
    svc.set("answer_base_url", "https://api.openai.com/v1")
    svc.set("answer_api_key", "sk-followup-test")
    svc.set("answer_model", "test-model")

    engine = AnswerEngine(Embedder(svc), svc)
    calls = []

    class _FakeClient:
        def __init__(self, *a, **kw):
            pass

        async def __aenter__(self):
            return self

        async def __aexit__(self, *a):
            return False

        async def post(self, url, headers=None, json=None):
            calls.append(json)
            msgs = json.get("messages", [])
            last = msgs[-1]["content"] if msgs else ""
            if "rewrite" in msgs[0]["content"].lower() if msgs else False:
                content = "What are the movement rules for the MRGR merger?"
            elif "Conversation so far" in last:
                content = "final answer about movements"
            else:
                content = "ok"

            class _Resp:
                def raise_for_status(self):
                    pass

                def json(self):
                    return {"choices": [{"message": {"content": content}}]}

            return _Resp()

    import app.answer_engine as ae

    original = ae.RetryClient
    ae.RetryClient = _FakeClient
    try:
        rewritten = asyncio.run(
            engine._rewrite_followup(
                "show the movements for it",
                [
                    {"role": "user", "content": "merger details"},
                    {"role": "assistant", "content": "MRGR is a merger event."},
                ],
            )
        )
    finally:
        ae.RetryClient = original

    assert rewritten == "What are the movement rules for the MRGR merger?"
    # the rewrite call carried the conversation transcript
    rewrite_payload = calls[0]
    assert "merger details" in rewrite_payload["messages"][-1]["content"]
    assert "show the movements for it" in rewrite_payload["messages"][-1]["content"]

    svc.clear("answer_base_url")
    svc.clear("answer_api_key")
    svc.clear("answer_model")


def test_rewrite_returns_none_without_key():
    """No key + non-local provider → rewrite falls back to raw question."""
    from app.answer_engine import AnswerEngine
    from app.embedder import Embedder

    svc = _make_svc()
    svc.set("answer_base_url", "https://api.openai.com/v1")
    svc.set("answer_api_key", "")

    engine = AnswerEngine(Embedder(svc), svc)
    result = asyncio.run(
        engine._rewrite_followup("for it?", [{"role": "user", "content": "matter details"}])
    )
    assert result is None
    svc.clear("answer_base_url")
    svc.clear("answer_api_key")


def test_generate_includes_conversation():
    """Generation payload must contain the conversation transcript."""
    from app.answer_engine import AnswerEngine
    from app.embedder import Embedder

    svc = _make_svc()
    svc.set("answer_base_url", "https://api.openai.com/v1")
    svc.set("answer_api_key", "sk-gen-test")

    engine = AnswerEngine(Embedder(svc), svc)
    captured = {}

    class _FakeClient:
        def __init__(self, *a, **kw):
            pass

        async def __aenter__(self):
            return self

        async def __aexit__(self, *a):
            return False

        async def post(self, url, headers=None, json=None):
            captured["payload"] = json

            class _Resp:
                def raise_for_status(self):
                    pass

                def json(self):
                    return {"choices": [{"message": {"content": "ok"}}]}

            return _Resp()

    import app.answer_engine as ae

    original = ae.RetryClient
    ae.RetryClient = _FakeClient
    try:
        asyncio.run(
            engine._generate(
                "show the movements for it",
                "[S1] context",
                history=[
                    {"role": "user", "content": "merger details"},
                    {"role": "assistant", "content": "MRGR is a merger event."},
                ],
            )
        )
    finally:
        ae.RetryClient = original

    user_content = captured["payload"]["messages"][-1]["content"]
    assert "Conversation so far" in user_content
    assert "merger details" in user_content
    assert "MRGR is a merger event." in user_content
    svc.clear("answer_base_url")
    svc.clear("answer_api_key")
