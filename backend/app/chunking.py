"""Chunking primitives (B35): text chunking + section-aware document chunking.

Extracted from pipeline.py so the ingestion orchestration stays thin and each
chunking strategy is unit-testable in isolation.
"""

import re

from .config import settings

__all__ = ["chunk_text", "chunk_document", "classify_windows"]


def chunk_text(text: str) -> list[str]:
    """Fixed-size chunks with overlap (used for entity summaries)."""
    size, overlap = settings.chunk_size, settings.chunk_overlap
    if len(text) <= size:
        return [text]
    step = size - overlap
    return [text[i : i + size] for i in range(0, max(len(text) - size + 1, 1), step)]


_HEADING_RE = re.compile(
    r"^\s*(?:"
    r"§\s*\d+(\.\d+)*"  # §434, §4.2.1
    r"|\d{1,4}(\.\d{1,4}){1,3}"  # 4.2.1, 12.3.4.5
    r"|(?:article|annex|section|schedule|rule|appendix)\s+\d+"  # Article 12
    r")(?:\s|[:.)\-]|$)",
    re.IGNORECASE,
)

_CAPS_HEADING_RE = re.compile(r"^[A-Z][A-Z0-9 &()/\-]{3,80}$")


def _is_heading(line: str) -> bool:
    """A line that looks like a section heading: numbered markers (§434,
    4.2.1, Article 12) or short ALL-CAPS titles. Page-number lines are
    already removed by cleaning before chunking."""
    stripped = line.strip()
    if not stripped or len(stripped) > 80:
        return False
    if _HEADING_RE.match(stripped):
        return True
    if _CAPS_HEADING_RE.match(stripped) and not stripped.isdigit():
        return True
    return False


def chunk_document(text: str) -> list[str]:
    """Section-aware chunks: hard boundaries at heading lines, paragraph
    accumulation inside a section, fixed-size fallback only for oversized
    paragraphs/sections. Max section size = chunk_max_chars."""
    max_chars = settings.chunk_max_chars
    lines = text.split("\n")
    sections: list[str] = []
    current: list[str] = []
    for line in lines:
        if _is_heading(line):
            if current:
                sections.append("\n".join(current))
                current = []
            current.append(line)
        else:
            current.append(line)
    if current:
        sections.append("\n".join(current))

    chunks: list[str] = []
    for section in sections:
        chunks.extend(_split_section(section, max_chars))
    return chunks


def _split_section(section: str, max_chars: int) -> list[str]:
    """Split one section by paragraphs; fall back to fixed-size for oversized
    paragraphs. Keeps headings attached to their section's first chunk."""
    if len(section) <= max_chars:
        return [section]
    paragraphs = [p.strip() for p in re.split(r"\n\s*\n", section) if p.strip()]
    out: list[str] = []
    current = ""
    for paragraph in paragraphs:
        if len(paragraph) > max_chars:
            if current:
                out.append(current)
                current = ""
            out.extend(chunk_text(paragraph))
        elif len(current) + len(paragraph) + 2 <= max_chars:
            current = paragraph if not current else f"{current}\n\n{paragraph}"
        else:
            if current:
                out.append(current)
            current = paragraph
    if current:
        out.append(current)
    return out


def classify_windows(text: str) -> list[str]:
    """Split long documents into overlapping windows for classification."""
    window = settings.classify_window_chars
    if len(text) <= window:
        return [text]
    step = max(window // 2, 1)
    return [text[i : i + window] for i in range(0, max(len(text) - window + 1, 1), step)]
