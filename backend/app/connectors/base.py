from dataclasses import dataclass, field
from datetime import datetime
from pathlib import Path


@dataclass
class IngestionDoc:
    external_id: str
    title: str
    text: str
    author: str = ""
    updated_at: datetime | None = None
    source_ref: str = ""
    extra: dict = field(default_factory=dict)


class ConnectorError(Exception):
    pass


class BaseConnector:
    connector_type = "base"

    def __init__(self, config: dict):
        self.config = config

    async def fetch(self, since_cursor: str = "") -> tuple[list[IngestionDoc], str]:
        """Return (new_or_updated_docs, next_cursor)."""
        raise NotImplementedError


def read_plain_text(path: Path) -> str:
    suffix = path.suffix.lower()
    if suffix in {".txt", ".md", ".markdown", ".text", ".csv", ".json"}:
        return path.read_text(encoding="utf-8", errors="replace")
    if suffix == ".pdf":
        from io import BytesIO

        from pypdf import PdfReader

        reader = PdfReader(BytesIO(path.read_bytes()))
        return "\n\n".join(page.extract_text() or "" for page in reader.pages)
    if suffix in {".html", ".htm"}:
        import re

        text = path.read_text(encoding="utf-8", errors="replace")
        text = re.sub(r"<script[^>]*>.*?</script>", " ", text, flags=re.IGNORECASE | re.DOTALL)
        text = re.sub(r"<style[^>]*>.*?</style>", " ", text, flags=re.IGNORECASE | re.DOTALL)
        text = re.sub(r"<[^>]+>", " ", text)
        return re.sub(r"\s+", " ", text).strip()
    raise ConnectorError(f"Unsupported file type: {suffix or 'none'}")
