import logging
import re

# Patterns applied to every log line and stored error message so credentials
# never reach logs, the UI, or exported files.
_PATTERNS = [
    # OpenAI-style keys: sk-...
    re.compile(r"\bsk-[A-Za-z0-9_-]{16,}\b"),
    # Authorization headers
    re.compile(r"\bBearer\s+[A-Za-z0-9._~+/-]+=*\b", re.IGNORECASE),
    re.compile(r"\bBasic\s+[A-Za-z0-9+/]{16,}={0,2}\b", re.IGNORECASE),
    # key=value / key: value pairs for common secret field names
    re.compile(
        r"\b(password|passwd|pwd|token|api[-_]?key|secret|authorization|credential)\b\s*[:=]\s*[^\s,;&'\"]+",
        re.IGNORECASE,
    ),
    # Jira-style API tokens: base64-ish long alnum blobs assigned to tokens
    re.compile(r"\btoken['\"]?\s*[:=]\s*['\"]?[A-Za-z0-9+/=]{24,}", re.IGNORECASE),
]

MASK = "***REDACTED***"


def redact(text: str | None) -> str:
    """Mask secret-like substrings in any user-visible text."""
    if not text:
        return text or ""
    out = text
    for pattern in _PATTERNS:
        out = pattern.sub(MASK, out)
    return out


class RedactingFormatter(logging.Formatter):
    """Formatter that masks secrets in the fully rendered log line."""

    def format(self, record: logging.LogRecord) -> str:
        return redact(super().format(record))
