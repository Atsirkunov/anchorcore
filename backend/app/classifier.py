import hashlib
import json
import logging
import re
from typing import Any

from .app_settings import SettingsService
from .http import RetryClient

logger = logging.getLogger(__name__)

KINDS = ["decision", "document", "action", "note"]

DOC_TYPES = ["standards", "runbook", "meeting", "decision_log", "prd", "general"]

# Per-document-type extraction prompts (B26): the extraction prompt adapts to
# what kind of document the text is, so e.g. a regulatory standard doesn't get
# spurious "decision"/"action" labels for procedural steps.
PROMPTS: dict[str, str] = {
    "general": """You extract knowledge items from organizational text.

Classify each item into exactly one of these kinds:
- decision: a choice made, with reasoning behind it
- document: a factual reference / status update / description
- action: something to be done, assigned, or tracked
- note: anything else worth remembering

Respond ONLY with a JSON object:
{{"items": [{{"kind": str, "summary": str, "reasoning": str, "confidence": float, "author": str}}]}}
Keep summaries to one sentence. confidence must be between 0 and 1. If no items exist, return {{"items": []}}.
""",
    "standards": """You extract knowledge items from a REGULATORY / STANDARDS document.

This document defines normative rules and procedures. Extract ONLY factual
statements worth remembering:
- note: a rule, requirement, definition, or procedural fact
Do NOT extract:
- decision (standards don't record organizational decisions)
- action (procedural steps are rules, not assigned to-dos)
- document (do not emit a summary for the document itself)

Respond ONLY with a JSON object:
{{"items": [{{"kind": "note", "summary": str, "reasoning": str, "confidence": float, "author": ""}}]}}
Keep summaries to one sentence. confidence must be between 0 and 1. If no items exist, return {{"items": []}}.
""",
    "runbook": """You extract knowledge items from a RUNBOOK / OPERATIONAL GUIDE.

Extract ONLY factual statements worth remembering:
- note: a step, prerequisite, escalation rule, or operational fact
Do NOT extract:
- decision (runbooks don't record organizational decisions)
- action (steps are procedural notes, not assigned to-dos)
- document (do not emit a summary for the document itself)

Respond ONLY with a JSON object:
{{"items": [{{"kind": "note", "summary": str, "reasoning": str, "confidence": float, "author": ""}}]}}
Keep summaries to one sentence. confidence must be between 0 and 1. If no items exist, return {{"items": []}}.
""",
    "meeting": """You extract knowledge items from MEETING NOTES.

Classify each item into exactly one of these kinds:
- decision: a choice made in the meeting, with reasoning
- action: something someone agreed to do (owner + due date if stated)
- document: a status update / factual report
- note: anything else worth remembering

Respond ONLY with a JSON object:
{{"items": [{{"kind": str, "summary": str, "reasoning": str, "confidence": float, "author": str}}]}}
Keep summaries to one sentence. confidence must be between 0 and 1. If no items exist, return {{"items": []}}.
""",
    "decision_log": """You extract knowledge items from a DECISION LOG.

Classify each item into exactly one of these kinds:
- decision: a recorded choice, with reasoning
- note: context, constraints, or revisit triggers
- action: a follow-up someone owns
Do NOT extract the document itself as an item.

Respond ONLY with a JSON object:
{{"items": [{{"kind": str, "summary": str, "reasoning": str, "confidence": float, "author": str}}]}}
Keep summaries to one sentence. confidence must be between 0 and 1. If no items exist, return {{"items": []}}.
""",
    "prd": """You extract knowledge items from a PRODUCT / REQUIREMENTS document.

Classify each item into exactly one of these kinds:
- decision: a scope or requirement choice, with reasoning
- document: a factual description / requirement / spec fact
- action: an explicitly assigned follow-up
- note: anything else worth remembering
Do NOT extract the document itself as an item.

Respond ONLY with a JSON object:
{{"items": [{{"kind": str, "summary": str, "reasoning": str, "confidence": float, "author": str}}]}}
Keep summaries to one sentence. confidence must be between 0 and 1. If no items exist, return {{"items": []}}.
""",
}

TYPE_DETECT_PROMPT = """You identify what kind of document a piece of text is.
Pick exactly one of: standards, runbook, meeting, decision_log, prd, general.
- standards: regulatory/normative rules, requirements, procedures
- runbook: operational steps, escalation rules, how-to guide
- meeting: meeting notes, retros, planning notes
- decision_log: a log of recorded decisions
- prd: product requirements, specifications, scoping docs
- general: anything else
Respond ONLY with the single word, nothing else.
"""

DISTILL_PROMPT = """You distill a conversation / chat-log excerpt into searchable Q&A units.

A unit is a question someone asked AND its substantive answer. Normalize the
exchange into a consistent format so it can be found later by either side.

For each distinct Q&A exchange in the text, output exactly one unit with:
- question: the question, normalized to a standalone one-liner
- answer: the substantive answer, normalized (no filler, greetings, or chit-chat)
- terms: 1-5 searchable key terms / acronyms the unit is about
- systems: any systems, teams, or components mentioned (empty if none)

Ignore filler: greetings, small talk, acknowledgements ("got it", "thanks"),
and messages with no information. If the text contains no real Q&A exchange,
return {{"units": []}}.

Respond ONLY with a JSON object:
{{"units": [{{"question": str, "answer": str, "terms": [str], "systems": [str]}}]}}
"""

# doc types where the distillation pass runs (chat-like content); standards and
# runbooks are already clean prose and don't need Q&A normalization
DISTILL_DOC_TYPES = {"meeting", "decision_log", "general"}


class Classifier:
    def __init__(self, settings: SettingsService):
        self.settings = settings

    async def detect_document_type(self, text: str) -> str:
        """Classify the document type once per document (1 call, cheap)."""
        try:
            return await self._detect_llm(text)
        except Exception as exc:  # noqa: BLE001
            logger.warning("document-type detection failed (%s); defaulting to general", exc)
            return "general"

    async def classify(self, text: str, source_ref: str, doc_type: str = "general") -> list[dict[str, Any]]:
        try:
            return await self._classify_llm(text, source_ref, doc_type)
        except Exception as exc:  # noqa: BLE001
            logger.warning("LLM classification failed (%s); falling back to rules", exc)
            return self._classify_rules(text, source_ref)

    async def distill(self, text: str, source_ref: str) -> list[dict[str, Any]]:
        """Distill a chat-like window into normalized Q&A units (B18).

        Returns a list of {question, answer, terms, systems} dicts. Falls back
        to a heuristic rule splitter when the LLM is unavailable (CI-safe)."""
        try:
            return await self._distill_llm(text, source_ref)
        except Exception as exc:  # noqa: BLE001
            logger.warning("LLM distillation failed (%s); falling back to rules", exc)
            return self._distill_rules(text)

    async def _distill_llm(self, text: str, source_ref: str) -> list[dict[str, Any]]:
        payload = {
            "model": self.settings.get("classifier_model") or "llama3.2:3b",
            "messages": [
                {"role": "system", "content": DISTILL_PROMPT},
                {"role": "user", "content": f"Source: {source_ref}\n\nContent:\n{text}"},
            ],
            "temperature": 0.0,
            "response_format": {"type": "json_object"},
        }
        base_url = self._base_url()
        api_key = self.settings.get("classifier_api_key") or ""
        headers = {"Authorization": f"Bearer {api_key}"} if api_key else {}
        timeout = self.settings.get_float("classifier_timeout", 60.0)
        async with RetryClient(timeout=timeout) as client:
            resp = await client.post(
                f"{base_url.rstrip('/')}/v1/chat/completions",
                headers=headers,
                json=payload,
            )
            resp.raise_for_status()
            raw = resp.json()["choices"][0]["message"]["content"]
            data = json.loads(raw)
        units: list[dict[str, Any]] = []
        for unit in data.get("units", []):
            if not isinstance(unit, dict):
                continue
            question = str(unit.get("question", "")).strip()
            answer = str(unit.get("answer", "")).strip()
            if not question or not answer:
                continue
            terms = [str(t).strip() for t in unit.get("terms", []) if str(t).strip()]
            systems = [str(s).strip() for s in unit.get("systems", []) if str(s).strip()]
            units.append(
                {
                    "question": question[:500],
                    "answer": answer[:1200],
                    "terms": terms[:5],
                    "systems": systems[:5],
                }
            )
        return units

    def _distill_rules(self, text: str) -> list[dict[str, Any]]:
        """Heuristic fallback: pair a question sentence with the following
        answer sentence, skipping greeting-only filler."""
        sentences = [s.strip() for s in re.split(r"(?<=[.!?])\s+", text) if s.strip()]
        units: list[dict[str, Any]] = []
        for i, sentence in enumerate(sentences):
            if not sentence.endswith("?") or i + 1 >= len(sentences):
                continue
            answer = sentences[i + 1]
            if len(answer) < 5 or re.fullmatch(r"(got it|ok|thanks|thx|yes|no|sure)[.!]*", answer, re.IGNORECASE):
                continue
            units.append(
                {
                    "question": sentence[:500],
                    "answer": answer[:1200],
                    "terms": [t.lower() for t in re.findall(r"[A-Za-z][A-Za-z0-9_\-]{2,}", sentence + " " + answer)[:5]],
                    "systems": [],
                }
            )
        return units

    async def _detect_llm(self, text: str) -> str:
        truncated = text[:8000]
        payload = {
            "model": self.settings.get("classifier_model") or "llama3.2:3b",
            "messages": [
                {"role": "system", "content": TYPE_DETECT_PROMPT},
                {"role": "user", "content": f"Document text (beginning):\n{truncated}"},
            ],
            "temperature": 0.0,
            "max_tokens": 10,
        }
        base_url = self._base_url()
        api_key = self.settings.get("classifier_api_key") or ""
        headers = {"Authorization": f"Bearer {api_key}"} if api_key else {}
        async with RetryClient(timeout=self.settings.get_float("classifier_timeout", 60.0)) as client:
            resp = await client.post(
                f"{base_url.rstrip('/')}/v1/chat/completions",
                headers=headers,
                json=payload,
            )
            resp.raise_for_status()
            raw = resp.json()["choices"][0]["message"]["content"].strip().lower()
        for doc_type in DOC_TYPES:
            if doc_type in raw:
                return doc_type
        return "general"

    async def _classify_llm(self, text: str, source_ref: str, doc_type: str) -> list[dict[str, Any]]:
        prompt = PROMPTS.get(doc_type, PROMPTS["general"])
        payload = {
            "model": self.settings.get("classifier_model") or "llama3.2:3b",
            "messages": [
                {"role": "system", "content": prompt},
                {"role": "user", "content": f"Source: {source_ref}\n\nContent:\n{text}"},
            ],
            "temperature": 0.1,
            "response_format": {"type": "json_object"},
        }
        base_url = self._base_url()
        api_key = self.settings.get("classifier_api_key") or ""
        headers = {"Authorization": f"Bearer {api_key}"} if api_key else {}
        timeout = self.settings.get_float("classifier_timeout", 60.0)
        async with RetryClient(timeout=timeout) as client:
            resp = await client.post(
                f"{base_url.rstrip('/')}/v1/chat/completions",
                headers=headers,
                json=payload,
            )
            resp.raise_for_status()
            raw = resp.json()["choices"][0]["message"]["content"]
            data = json.loads(raw)
        return self._normalize(data.get("items", []), source_ref)

    def _base_url(self) -> str:
        """Classifier endpoint: explicit classifier_base_url wins; otherwise
        the shared Ollama base URL (local default)."""
        return (
            self.settings.get("classifier_base_url")
            or self.settings.get("ollama_base_url")
            or "http://localhost:11434"
        )

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


def window_hash(text: str) -> str:
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def _clamp_float(value: Any, default: float) -> float:
    try:
        f = float(value)
    except (TypeError, ValueError):
        return default
    return max(0.0, min(1.0, f))
