"""Shared health/status probes used by the /health and /system endpoints."""

import httpx

from .config import settings


def ollama_reachable() -> bool:
    try:
        with httpx.Client(timeout=httpx.Timeout(3.0, connect=2.0)) as client:
            resp = client.get(f"{settings.ollama_base_url.rstrip('/')}/api/tags")
            return resp.status_code == 200
    except httpx.TransportError:
        return False


def missing_ollama_models() -> list[str]:
    """Configured models (classifier + embed) absent from Ollama's tags.

    Compares base names so "nomic-embed-text" matches the ":latest" tag.
    Returns both models when Ollama is unreachable.
    """
    try:
        with httpx.Client(timeout=httpx.Timeout(3.0, connect=2.0)) as client:
            resp = client.get(f"{settings.ollama_base_url.rstrip('/')}/api/tags")
            if resp.status_code != 200:
                return [settings.classifier_model, settings.embed_model]
    except httpx.TransportError:
        return [settings.classifier_model, settings.embed_model]
    tags = {m.get("name", "").split(":", 1)[0] for m in resp.json().get("models", [])}
    missing = []
    for model in (settings.classifier_model, settings.embed_model):
        if model.split(":", 1)[0] not in tags:
            missing.append(model)
    return missing


def answer_provider() -> str:
    if settings.answer_base_url.startswith(("http://localhost", "http://127.0.0.1")):
        return "ollama"
    if settings.answer_api_key:
        return "configured"
    return "missing"
