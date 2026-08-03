import logging
from datetime import datetime, timezone
from pathlib import Path

from .base import BaseConnector, ConnectorError, IngestionDoc, read_plain_text

logger = logging.getLogger(__name__)

SUPPORTED = {".txt", ".md", ".markdown", ".text", ".csv", ".json", ".pdf", ".html", ".htm"}


class FolderConnector(BaseConnector):
    connector_type = "folder"

    def __init__(self, config: dict):
        super().__init__(config)
        path = config.get("path")
        if not path:
            raise ConnectorError("folder connector requires 'path'")
        self.path = Path(path).expanduser().resolve()
        if not self.path.is_dir():
            raise ConnectorError(f"folder does not exist: {self.path}")

    async def fetch(self, since_cursor: str = "") -> tuple[list[IngestionDoc], str]:
        seen = []
        for file in self.path.rglob("*"):
            if not file.is_file() or file.suffix.lower() not in SUPPORTED:
                continue
            try:
                text = read_plain_text(file)
            except ConnectorError:
                continue
            mtime = datetime.fromtimestamp(file.stat().st_mtime, tz=timezone.utc)
            seen.append(
                IngestionDoc(
                    external_id=str(file.relative_to(self.path)),
                    title=file.name,
                    text=text,
                    source_ref=str(file),
                    updated_at=mtime,
                )
            )
        return seen, ""
