"""Shared health/status probes used by the /health and /system endpoints.

Reads effective settings through the SettingsService so status reflects
runtime changes without restart (B4).
"""

import os

import httpx

from .app_settings import SettingsService
from .config import settings


def _probe_timeout() -> httpx.Timeout:
    return httpx.Timeout(3.0, connect=settings.http_connect_timeout)


def ollama_reachable(settings: SettingsService) -> bool:
    base_url = settings.get("ollama_base_url") or "http://localhost:11434"
    try:
        with httpx.Client(timeout=_probe_timeout()) as client:
            resp = client.get(f"{base_url.rstrip('/')}/api/tags")
            return resp.status_code == 200
    except httpx.TransportError:
        return False


def classifier_is_local(settings: SettingsService) -> bool:
    base_url = settings.get("classifier_base_url") or settings.get("ollama_base_url") or ""
    return base_url.startswith(("http://localhost", "http://127.0.0.1"))


def embedder_is_local(settings: SettingsService) -> bool:
    base_url = settings.get("embed_base_url") or settings.get("ollama_base_url") or ""
    return base_url.startswith(("http://localhost", "http://127.0.0.1"))


def missing_ollama_models(settings: SettingsService) -> list[str]:
    """Configured models (classifier + embed) absent from Ollama's tags.

    Compares base names so "nomic-embed-text" matches the ":latest" tag.
    Returns both models when Ollama is unreachable. When a provider runs on
    cloud (B23), only the models that actually use Ollama are checked.
    """
    base_url = settings.get("ollama_base_url") or "http://localhost:11434"
    classifier_model = (
        (settings.get("classifier_model") or "llama3.2:3b") if classifier_is_local(settings) else None
    )
    embed_model = (settings.get("embed_model") or "nomic-embed-text") if embedder_is_local(settings) else None
    try:
        with httpx.Client(timeout=_probe_timeout()) as client:
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


# B39: provider trust. Default policy: local (Ollama / 127.0.0.1) providers are
# trusted; a remote provider is only trusted when the user explicitly confirms
# it (ANCHOR_CLOUD_TRUST=1). Until confirmed, sensitive/pii sources never leave
# the machine — cloud classify/embed is refused and falls back to rule-based.
_SENSITIVE_LABELS = {"sensitive", "pii"}


def cloud_classifier_trusted(settings: SettingsService) -> bool:
    if classifier_is_local(settings):
        return True
    return os.environ.get("ANCHOR_CLOUD_TRUST") == "1"


def cloud_embedder_trusted(settings: SettingsService) -> bool:
    if embedder_is_local(settings):
        return True
    return os.environ.get("ANCHOR_CLOUD_TRUST") == "1"


def cloud_answer_trusted(settings: SettingsService) -> bool:
    """B30: answer-provider trust. A local (Ollama / 127.0.0.1) answer
    provider is always trusted; a cloud answer provider needs the user to
    confirm ANCHOR_CLOUD_TRUST=1 before sensitive/pii content is sent to it."""
    base_url = settings.get("answer_base_url") or ""
    if base_url.startswith(("http://localhost", "http://127.0.0.1")):
        return True
    return os.environ.get("ANCHOR_CLOUD_TRUST") == "1"


def sensitive_label(label: str) -> bool:
    return label in _SENSITIVE_LABELS
