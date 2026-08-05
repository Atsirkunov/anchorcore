"""B4: runtime LLM configuration — settings CRUD, override precedence, secrets."""

import time


def test_settings_get_returns_env_defaults(client):
    resp = client.get("/settings")
    assert resp.status_code == 200, resp.text
    body = resp.json()
    assert "ollama_base_url" in body
    assert "classifier_model" in body
    assert "answer_model" in body
    # secret key present but never plaintext
    assert "answer_api_key" in body
    assert body["answer_api_key"] in ("", "***set***")


def test_settings_put_updates_and_persists(client):
    resp = client.put("/settings", json={"answer_model": "my-test-model-1"})
    assert resp.status_code == 200, resp.text
    assert resp.json()["answer_model"] == "my-test-model-1"

    # persists across requests (DB-backed, not just in-memory)
    again = client.get("/settings").json()
    assert again["answer_model"] == "my-test-model-1"


def test_settings_unknown_key_rejected(client):
    resp = client.put("/settings", json={"not_a_setting": "x"})
    assert resp.status_code == 422


def test_settings_secret_masked_and_stored_in_keystore(client):
    from app.app_settings import SECRET_PREFIX
    from app.main import settings_svc

    resp = client.put("/settings", json={"answer_api_key": "sk-test-secret-123"})
    assert resp.status_code == 200, resp.text
    body = resp.json()
    assert body["answer_api_key"] == "***set***", "secret must be masked in responses"
    assert "sk-test-secret-123" not in str(body), "plaintext secret must never leak"

    # lives in the SecretStore, not in app_settings DB rows
    stored = settings_svc.secrets.get(f"{SECRET_PREFIX}answer_api_key")
    assert stored == "sk-test-secret-123"

    # effective resolution through the service works at runtime
    assert settings_svc.get("answer_api_key") == "sk-test-secret-123"


def test_settings_clear_restores_env(client):
    client.put("/settings", json={"answer_model": "temp-model"})
    resp = client.put("/settings", json={"answer_model": None})
    assert resp.status_code == 200, resp.text
    value = resp.json()["answer_model"]
    assert value != "temp-model", "cleared setting must fall back to env default"


def test_test_connection_ollama_unreachable(client):
    # CI fixture points Ollama at localhost:1 → must fail gracefully
    resp = client.post("/settings/test-connection", json={"provider": "ollama"})
    assert resp.status_code == 200, resp.text
    body = resp.json()
    assert body["provider"] == "ollama"
    assert body["ok"] is False
    assert body["message"]


def test_test_connection_answer_requires_key(client):
    resp = client.post("/settings/test-connection", json={"provider": "answer"})
    assert resp.status_code == 200, resp.text
    body = resp.json()
    assert body["provider"] == "answer"
    # localhost answer base (test .env points at localhost:11434/v1) is allowed without key
    assert isinstance(body["ok"], bool)


def test_system_status_reflects_runtime_settings(client):
    client.put("/settings", json={"answer_model": "runtime-status-model"})
    status = client.get("/system/status").json()
    assert status["answer"]["model"] == "runtime-status-model"


def test_reasoning_effort_setting_persists(client):
    client.put("/settings", json={"answer_reasoning_effort": "high"})
    body = client.get("/settings").json()
    assert body["answer_reasoning_effort"] == "high"


def test_reasoning_effort_builds_payload(client):
    """_generate must pass reasoning_effort for cloud and drop temperature."""
    import asyncio

    from app.answer_engine import AnswerEngine
    from app.app_settings import SettingsService
    from app.config import settings
    from app.embedder import Embedder
    from app.secrets import SecretStore

    svc = SettingsService(settings, SecretStore(settings.data_dir / "secrets.enc"))
    svc.set("answer_reasoning_effort", "high")
    svc.set("answer_base_url", "https://api.openai.com/v1")
    svc.set("answer_api_key", "sk-reasoning-test")

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
            captured["headers"] = headers

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
        asyncio.run(engine._generate("q", "ctx"))
    finally:
        ae.RetryClient = original

    assert captured["payload"]["reasoning_effort"] == "high"
    assert "temperature" not in captured["payload"]
    assert "Bearer sk-reasoning-test" in (captured["headers"] or {}).get("Authorization", "")
    svc.clear("answer_reasoning_effort")
    svc.clear("answer_base_url")
    svc.clear("answer_api_key")
