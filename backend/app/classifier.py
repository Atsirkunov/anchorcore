import json
import logging
import re
from typing import Any

import httpx

from .config import settings

logger = logging.getLogger(__name__)

KINDS = ["decision", "document", "action", "note"]

SYSTEM_PROMPT = f"""You extract knowledge items from organizational documents.

Classify each item into exactly one of these kinds:
- decision: a choice made, with reasoning behind it
- document: a factual reference / status update / description
- action: something to be done, assigned, or tracked
- note: anything else worth remembering

Respond ONLY with a JSON object:
{{"items": [{{"kind": str, "summary": str, "reasoning": str, "confidence": float, "author": str, "source": str}}]}}
Keep summaries to one sentence. confidence must be between 0 and 1. If no items exist, return {{"items": []}}.
"""


class Classifier:
    async def classify(self, text: str, filename: str) -> list[dict[str, Any]]:
        if settings.classifier_base_url:
            try:
                return await self._classify_llm(text, filename)
            except Exception as exc:  # noqa: BLE001
                logger.warning("LLM classification failed (%s); falling back to rules", exc)
        return self._classify_rules(text, filename)

    async def _classify_llm(self, text: str, filename: str) -> list[dict[str, Any]]:
        truncated = text[:12000] if len(text) > 12000 else text
        payload = {
            "model": settings.classifier_model,
            "messages": [
                {"role": "system", "content": SYSTEM_PROMPT},
                {"role": "user", "content": f"File: {filename}\n\nDocument content:\n{truncated}"},
            ],
            "temperature": 0.1,
            "response_format": {"type": "json_object"},
        }
        async with httpx.AsyncClient(timeout=settings.classifier_timeout) as client:
            resp = await client.post(
                f"{settings.classifier_base_url.rstrip('/')}/chat/completions",
                headers={"Authorization": f"Bearer {settings.classifier_api_key}"},
                json=payload,
            )
            resp.raise_for_status()
            raw = resp.json()["choices"][0]["message"]["content"]
            data = json.loads(raw)
        items = data.get("items", [])
        return self._normalize(items, filename)

    def _classify_rules(self, text: str, filename: str) -> list[dict[str, Any]]:
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
                    "source": filename,
                }
            )
        return items

    def _rule_kind(self, sentence: str) -> str | None:
        lower = sentence.lower()
        if re.search(r"\b(we (should|will|must|need to|decided)|decision|decided to|approved|rejected)\b", lower):
            return "decision"
        if re.search(r"\b(action item|todo|to-do|next step|owner|assign|follow[- ]up|deadline|open question)\b", lower):
            return "action"
        if re.search(r"\b(status|update|summary|as of|current state|is now|has been)\b", lower):
            return "document"
        return None

    def _normalize(self, items: list[dict[str, Any]], filename: str) -> list[dict[str, Any]]:
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
                    "confidence": _clamp_float(item.get("confidence")),
                    "author": str(item.get("author", "")).strip()[:300],
                    "source": str(item.get("source") or filename).strip()[:300],
                }
            )
        return [item for item in normalized if item["summary"]]


def _clamp_float(value: Any) -> float:
    try:
        f = float(value)
    except (TypeError, ValueError):
        return 0.0
    return max(0.0, min(1.0, f))
