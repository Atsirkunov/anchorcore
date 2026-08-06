"""Shared health/status probes used by the /health and /system endpoints.

Reads effective settings through the SettingsService so status reflects
runtime changes without restart (B4).
"""

import httpx

from .app_settings import SettingsService


def ollama_reachable(settings: SettingsService) -> bool:
    base_url = settings.get("ollama_base_url") or "http://localhost:11434"
    try:
        with httpx.Client(timeout=httpx.Timeout(3.0, connect=2.0)) as client:
            resp = client.get(f"{base_url.rstrip('/')}/api/tags")
            return resp.status_code == 200
    except httpx.TransportError:
        return False


def classifier_is_local(settings: SettingsService) -> bool:
    base_url = settings.get("classifier_base_url") or settings.get("ollama_base_url") or ""
    return base_url.startswith(("http://localhost", "http://127.0.0.1"))


def missing_ollama_models(settings: SettingsService) -> list[str]:
    """Configured models (classifier + embed) absent from Ollama's tags.

    Compares base names so "nomic-embed-text" matches the ":latest" tag.
    Returns both models when Ollama is unreachable. When the classifier runs
    on a cloud provider (B23), only the embed model is checked against Ollama.
    """
    embed_model = settings.get("embed_model") or "nomic-embed-text"
    base_url = settings.get("ollama_base_url") or "http://localhost:11434"
    classifier_model = (
        settings.get("classifier_model") or "llama3.2:3b"
        if classifier_is_local(settings)
        else None
    )
    try:
        with httpx.Client(timeout=httpx.Timeout(3.0, connect=2.0)) as client:
            resp = client.get(f"{base_url.rstrip('/')}/api/tags")
            if resp.status_code != 200:
                return [m for m in (classifier_model, embed_model) if m]
    except httpx.TransportError:
        return [m for m in (classifier_model, embed_model) if m]
    tags = {m.get("name", "").split(":", 1)[0] for m in resp.json().get("models", [])}
    missing = []
    for model in (classifier_model, embed_model):
        if model and model.split(":", 1)[0] not in tags:
            missing.append(model)
    return missing


def answer_provider(settings: SettingsService) -> str:
    base_url = settings.get("answer_base_url") or ""
    if base_url.startswith(("http://localhost", "http://127.0.0.1")):
        return "ollama"
    if settings.get("answer_api_key"):
        return "configured"
    return "missing"
