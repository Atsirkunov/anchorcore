import json
import logging
import re
from typing import Any

import httpx

from .app_settings import SettingsService
from .http import RetryClient

logger = logging.getLogger(__name__)

KINDS = ["decision", "document", "action", "note"]

SYSTEM_PROMPT = f"""You extract knowledge items from organizational text.

Classify each item into exactly one of these kinds:
- decision: a choice made, with reasoning behind it
- document: a factual reference / status update / description
- action: something to be done, assigned, or tracked
- note: anything else worth remembering

Respond ONLY with a JSON object:
{{"items": [{{"kind": str, "summary": str, "reasoning": str, "confidence": float, "author": str}}]}}
Keep summaries to one sentence. confidence must be between 0 and 1. If no items exist, return {{"items": []}}.
"""


class Classifier:
    def __init__(self, settings: SettingsService):
        self.settings = settings

    async def classify(self, text: str, source_ref: str) -> list[dict[str, Any]]:
        try:
            return await self._classify_llm(text, source_ref)
        except Exception as exc:  # noqa: BLE001
            logger.warning("LLM classification failed (%s); falling back to rules", exc)
            return self._classify_rules(text, source_ref)

    async def _classify_llm(self, text: str, source_ref: str) -> list[dict[str, Any]]:
        truncated = text[:12000] if len(text) > 12000 else text
        payload = {
            "model": self.settings.get("classifier_model") or "llama3.2:3b",
            "messages": [
                {"role": "system", "content": SYSTEM_PROMPT},
                {"role": "user", "content": f"Source: {source_ref}\n\nContent:\n{truncated}"},
            ],
            "temperature": 0.1,
            "response_format": {"type": "json_object"},
        }
        base_url = self.settings.get("ollama_base_url") or "http://localhost:11434"
        timeout = self.settings.get_float("classifier_timeout", 60.0)
        async with RetryClient(timeout=timeout) as client:
            resp = await client.post(
                f"{base_url.rstrip('/')}/v1/chat/completions",
                json=payload,
            )
            resp.raise_for_status()
            raw = resp.json()["choices"][0]["message"]["content"]
            data = json.loads(raw)
        return self._normalize(data.get("items", []), source_ref)

    def _classify_rules(self, text: str, source_ref: str) -> list[dict[str, Any]]:
        items: list[dict[str, Any]] = []
        for sentence in re.split(r"(?<=[.!?])\s+", text):
            sentence = sentence.strip()
            if not sentence:
                continue
            kind = self._rule_kind(sentence)
            if not kind:
                continue
            items.append(
                {
                    "kind": kind,
                    "summary": sentence[:240],
                    "reasoning": "rule-based classifier",
                    "confidence": 0.5,
                    "author": "",
                }
            )
        return self._normalize(items, source_ref)

    def _rule_kind(self, sentence: str) -> str | None:
        lower = sentence.lower()
        if re.search(r"\b(we (should|will|must|need to|decided)|decision|decided to|approved|rejected|postpone)\b", lower):
            return "decision"
        if re.search(r"\b(action item|todo|to-do|next step|owner|assign|follow[- ]up|deadline|due)\b", lower):
            return "action"
        if re.search(r"\b(status|update|summary|as of|current state|is now|has been)\b", lower):
            return "document"
        return None

    def _normalize(self, items: list[dict[str, Any]], source_ref: str) -> list[dict[str, Any]]:
        normalized = []
        for item in items:
            if not isinstance(item, dict):
                continue
            kind = str(item.get("kind", "note")).lower()
            if kind not in KINDS:
                kind = "note"
            normalized.append(
                {
                    "kind": kind,
                    "summary": str(item.get("summary", "")).strip()[:1000],
                    "reasoning": str(item.get("reasoning", "")).strip()[:1000],
                    "confidence": _clamp_float(item.get("confidence"), 0.5),
                    "author": str(item.get("author", "")).strip()[:300],
                    "source_ref": source_ref,
                }
            )
        return [item for item in normalized if item["summary"]]


def _clamp_float(value: Any, default: float) -> float:
    try:
        f = float(value)
    except (TypeError, ValueError):
        return default
    return max(0.0, min(1.0, f))
