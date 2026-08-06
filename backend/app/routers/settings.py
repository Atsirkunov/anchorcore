"""Runtime LLM configuration (B4): Settings tab backend.

GET  /settings          — effective values, secrets masked
PUT  /settings          — {key: value, ...}; secrets → SecretStore
POST /settings/test-connection — verify a provider works
"""

import logging

import httpx
from fastapi import APIRouter, Depends, HTTPException
from sqlalchemy.orm import Session

from ..app_settings import ALL_KEYS, SECRET_KEYS, SETTING_KEYS, SettingsService
from ..db import get_db

logger = logging.getLogger(__name__)

_PROVIDER_DEFAULTS = {
    "ollama": {
        "ollama_base_url": "http://localhost:11434",
        "classifier_model": "llama3.2:3b",
        "embed_model": "nomic-embed-text",
    },
    "openai": {
        "answer_base_url": "https://api.openai.com/v1",
        "answer_model": "gpt-4o-mini",
    },
}


def make_router(settings_svc: SettingsService) -> APIRouter:
    router = APIRouter(prefix="/settings", tags=["settings"])

    @router.get("")
    def get_settings(db: Session = Depends(get_db)) -> dict:
        return settings_svc.snapshot(db=db)

    @router.put("")
    def update_settings(payload: dict, db: Session = Depends(get_db)) -> dict:
        unknown = [k for k in payload if k not in ALL_KEYS]
        if unknown:
            raise HTTPException(status_code=422, detail=f"unknown settings: {', '.join(unknown)}")
        for key, value in payload.items():
            if value is None:
                settings_svc.clear(key, db=db)
            else:
                settings_svc.set(key, str(value), db=db)
        return settings_svc.snapshot(db=db)

    @router.post("/test-connection")
    async def test_connection(payload: dict) -> dict:
        provider = payload.get("provider", "ollama")
        if provider == "ollama":
            return await _test_ollama(settings_svc)
        if provider == "classifier":
            return await _test_classifier(settings_svc)
        if provider == "answer":
            return await _test_answer(settings_svc)
        raise HTTPException(status_code=422, detail=f"unknown provider: {provider}")

    return router


async def _test_classifier(settings_svc: SettingsService) -> dict:
    """Verify the classifier provider (local Ollama or cloud OpenAI-compatible)."""
    from ..classifier import Classifier

    classifier = Classifier(settings_svc)
    base_url = classifier._base_url()
    is_local = base_url.startswith(("http://localhost", "http://127.0.0.1"))
    api_key = settings_svc.get("classifier_api_key") or ""
    if not is_local and not api_key:
        return {
            "ok": False,
            "provider": "classifier",
            "message": "API key required for a cloud classifier provider",
        }
    try:
        import httpx

        headers = {"Authorization": f"Bearer {api_key}"} if api_key else {}
        async with httpx.AsyncClient(timeout=httpx.Timeout(10.0, connect=5.0)) as client:
            resp = await client.post(
                f"{base_url.rstrip('/')}/v1/chat/completions",
                headers=headers,
                json={
                    "model": settings_svc.get("classifier_model") or "llama3.2:3b",
                    "messages": [{"role": "user", "content": "ping"}],
                    "max_tokens": 1,
                },
            )
            resp.raise_for_status()
    except Exception as exc:  # noqa: BLE001
        return {
            "ok": False,
            "provider": "classifier",
            "message": f"request failed: {type(exc).__name__}",
        }
    return {"ok": True, "provider": "classifier", "message": "classifier provider responds"}


async def _test_ollama(settings_svc: SettingsService) -> dict:
    base_url = settings_svc.get("ollama_base_url") or _PROVIDER_DEFAULTS["ollama"]["ollama_base_url"]
    try:
        async with httpx.AsyncClient(timeout=httpx.Timeout(5.0, connect=3.0)) as client:
            resp = await client.get(f"{base_url.rstrip('/')}/api/tags")
            resp.raise_for_status()
            models = {m.get("name", "").split(":", 1)[0] for m in resp.json().get("models", [])}
    except Exception as exc:  # noqa: BLE001
        return {"ok": False, "provider": "ollama", "message": f"unreachable: {type(exc).__name__}"}

    classifier = (settings_svc.get("classifier_model") or "llama3.2:3b").split(":", 1)[0]
    embed = (settings_svc.get("embed_model") or "nomic-embed-text").split(":", 1)[0]
    missing = [m for m in (classifier, embed) if m not in models]
    if missing:
        return {
            "ok": False,
            "provider": "ollama",
            "message": f"reachable; models missing: {', '.join(missing)} (run: ollama pull {' && ollama pull '.join(missing)})",
        }
    return {"ok": True, "provider": "ollama", "message": "Ollama up, models present"}


async def _test_answer(settings_svc: SettingsService) -> dict:
    base_url = settings_svc.get("answer_base_url") or ""
    api_key = settings_svc.get("answer_api_key") or ""
    model = settings_svc.get("answer_model") or "gpt-4o-mini"
    if not base_url:
        return {"ok": False, "provider": "answer", "message": "no answer base URL configured"}

    is_local = base_url.startswith(("http://localhost", "http://127.0.0.1"))
    if not is_local and not api_key:
        return {"ok": False, "provider": "answer", "message": "API key required for this provider"}

    headers = {"Authorization": f"Bearer {api_key}"} if api_key else {}
    try:
        async with httpx.AsyncClient(timeout=httpx.Timeout(15.0, connect=5.0)) as client:
            resp = await client.post(
                f"{base_url.rstrip('/')}/chat/completions",
                headers=headers,
                json={
                    "model": model,
                    "messages": [{"role": "user", "content": "ping"}],
                    "max_tokens": 1,
                },
            )
            resp.raise_for_status()
    except Exception as exc:  # noqa: BLE001
        return {
            "ok": False,
            "provider": "answer",
            "message": f"request failed: {type(exc).__name__}",
        }
    return {"ok": True, "provider": "answer", "message": f"model '{model}' responds"}


def provider_presets() -> dict:
    return _PROVIDER_DEFAULTS
