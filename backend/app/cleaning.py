"""Chunk text cleaning for retrieval quality (B12).

Strips encoding artifacts, page numbers, repeated headers/footers, and
normalizes whitespace so embedded text and FTS tokens are clean.
"""

import re

_CONTROL_RE = re.compile(
    r"[\x00-\x08\x0b\x0c\x0e-\x1f\x7f\u00ad\u200b-\u200f\ufeff\ufffd]"
)
_PAGE_NUM_RE = re.compile(r"^(?:page\s*)?\d{1,4}(?:\s*of\s*\d{1,4})?\s*$", re.IGNORECASE)
_ORPHAN_PAGE_NUM_RE = re.compile(r"^[-–—]\s*\d{1,4}\s*[-–—]$")
_WS_RE = re.compile(r"[ \t]+")


def clean_text(text: str) -> str:
    """Per-chunk cleaning: control chars, page-number lines, whitespace."""
    text = text.replace("\r\n", "\n").replace("\r", "\n")
    text = _CONTROL_RE.sub("", text)
    out: list[str] = []
    for line in text.split("\n"):
        stripped = line.strip()
        if not stripped:
            out.append("")
            continue
        if _PAGE_NUM_RE.match(stripped) or _ORPHAN_PAGE_NUM_RE.match(stripped):
            continue
        out.append(_WS_RE.sub(" ", line).rstrip())
    # collapse runs of blank lines to a single one (paragraph separator)
    result: list[str] = []
    blanks = 0
    for line in out:
        if not line:
            blanks += 1
            if blanks > 1:
                continue
        else:
            blanks = 0
        result.append(line)
    return "\n".join(result).strip()


def repeated_lines(text: str, min_repeats: int | None = None) -> set[str]:
    """Lines repeated across the document (headers/footers/templates).

    A line that appears many times verbatim is almost certainly boilerplate,
    not content. Returns lowercased stripped lines to drop.
    """
    counts: dict[str, int] = {}
    total = 0
    for line in text.split("\n"):
        s = line.strip().lower()
        if not s or len(s) > 120:
            continue
        counts[s] = counts.get(s, 0) + 1
        total += 1
    threshold = min_repeats if min_repeats is not None else max(3, round(total * 0.01))
    return {s for s, count in counts.items() if count >= threshold}


def strip_repeated(text: str, repeated: set[str]) -> str:
    """Remove lines that belong to the repeated (boilerplate) set."""
    if not repeated:
        return text
    return "\n".join(
        line for line in text.split("\n") if line.strip().lower() not in repeated
    )
