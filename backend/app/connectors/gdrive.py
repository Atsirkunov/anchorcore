import logging
from datetime import datetime, timezone
from urllib.parse import quote

from ..http import RetryClient
from .base import BaseConnector, ConnectorError, IngestionDoc

logger = logging.getLogger(__name__)

# Google-native mime types → export mime for text extraction
_NATIVE_EXPORT = {
    "application/vnd.google-apps.document": "text/plain",
    "application/vnd.google-apps.presentation": "text/plain",
    "application/vnd.google-apps.spreadsheet": "text/csv",
    # drawings etc. fall back to pdf export if needed
}
_NATIVE_MIME = set(_NATIVE_EXPORT)

# drive API base
_DRIVE_BASE = "https://www.googleapis.com/drive/v3"
# fields we request for list
_LIST_FIELDS = "nextPageToken, files(id, name, mimeType, modifiedTime, size, parents, trashed)"


class GDriveConnector(BaseConnector):
    """Google Drive folder → IngestionDocs (B28 simple path: direct ingest, no local mirror).

    Config (non-secret): folder_id (Drive folder ID, required), drive_id (optional shared drive).
    Secret (via SecretStore field `token`): OAuth access token (Bearer).

    Incremental: cursor is ISO modifiedTime of the newest file seen (like Jira).
    Listing uses `modifiedTime > cursor` + `'<folder_id>' in parents`.

    Tests mock RetryClient — no real credentials needed in CI (see tests/test_gdrive.py).
    """

    connector_type = "gdrive"

    def __init__(self, config: dict):
        super().__init__(config)
        self.folder_id = (config.get("folder_id") or config.get("folder") or "").strip()
        self.token = (config.get("token") or config.get("access_token") or "").strip()
        # optional shared-drive
        self.drive_id = (config.get("drive_id") or "").strip()
        if not self.folder_id:
            raise ConnectorError("gdrive connector requires folder_id")
        if not self.token:
            raise ConnectorError("gdrive connector requires token (OAuth access token)")

    def _headers(self) -> dict[str, str]:
        return {"Authorization": f"Bearer {self.token}"}

    async def fetch(self, since_cursor: str = "") -> tuple[list[IngestionDoc], str]:
        docs: list[IngestionDoc] = []
        next_cursor = since_cursor
        page_token = ""

        # Drive query: files in folder, not trashed, optionally modified after cursor
        # Note: cursor is modifiedTime ISO. Drive expects RFC3339; since_cursor is ISO from updated_at.
        q_parts = [f"'{self.folder_id}' in parents", "trashed = false"]
        if since_cursor:
            # Drive query needs quoted RFC3339; ensure Z suffix
            q_parts.append(f"modifiedTime > '{since_cursor}'")
        q = " and ".join(q_parts)

        async with RetryClient(timeout=60.0) as client:
            while True:
                params: dict = {
                    "q": q,
                    "fields": _LIST_FIELDS,
                    "pageSize": 100,
                    "orderBy": "modifiedTime asc",
                    "includeItemsFromAllDrives": "true" if self.drive_id else "false",
                    "supportsAllDrives": "true" if self.drive_id else "false",
                }
                if page_token:
                    params["pageToken"] = page_token

                resp = await client.get(f"{_DRIVE_BASE}/files", headers=self._headers(), params=params)
                if resp.status_code == 401:
                    raise ConnectorError("Drive auth failed (401) — token expired or invalid; re-authorize in Sources")
                resp.raise_for_status()
                data = resp.json()

                for f in data.get("files", []):
                    mime = f.get("mimeType") or ""
                    # skip folders
                    if mime == "application/vnd.google-apps.folder":
                        continue
                    file_id = f.get("id")
                    name = f.get("name") or file_id
                    modified_raw = f.get("modifiedTime") or ""
                    updated_at = None
                    if modified_raw:
                        try:
                            updated_at = datetime.fromisoformat(modified_raw.replace("Z", "+00:00"))
                        except ValueError:
                            updated_at = None

                    # download / export content
                    try:
                        text = await self._download_file(client, file_id, mime, name)
                    except Exception as exc:  # noqa: BLE001
                        logger.warning("gdrive skip %s (%s): %s", name, file_id, exc)
                        continue

                    if not text or not text.strip():
                        continue

                    docs.append(
                        IngestionDoc(
                            external_id=file_id,
                            title=name,
                            text=text,
                            author="",
                            updated_at=updated_at,
                            source_ref=f"Drive:{name}",
                        )
                    )
                    if updated_at and (not next_cursor or updated_at.isoformat() > next_cursor):
                        next_cursor = updated_at.isoformat()

                page_token = data.get("nextPageToken") or ""
                if not page_token:
                    break

        return docs, next_cursor

    async def _download_file(self, client: RetryClient, file_id: str, mime: str, name: str) -> str:
        # Google-native → export
        if mime in _NATIVE_MIME:
            export_mime = _NATIVE_EXPORT[mime]
            url = f"{_DRIVE_BASE}/files/{quote(file_id)}/export"
            resp = await client.get(url, headers=self._headers(), params={"mimeType": export_mime})
            resp.raise_for_status()
            return resp.text

        # regular file → alt=media (bytes)
        url = f"{_DRIVE_BASE}/files/{quote(file_id)}"
        resp = await client.get(url, headers=self._headers(), params={"alt": "media"})
        # Drive returns 200 with file bytes; for text we decode, for pdf we extract
        resp.raise_for_status()
        ct = (resp.headers.get("content-type") or "").lower()
        body = resp.content

        # pdf
        if mime == "application/pdf" or name.lower().endswith(".pdf") or ct == "application/pdf":
            from io import BytesIO

            from pypdf import PdfReader

            reader = PdfReader(BytesIO(body))
            return "\n\n".join(page.extract_text() or "" for page in reader.pages)

        # html
        if mime == "text/html" or name.lower().endswith((".html", ".htm")) or "text/html" in ct:
            import re

            text = body.decode("utf-8", errors="replace")
            text = re.sub(r"<script[^>]*>.*?</script>", " ", text, flags=re.IGNORECASE | re.DOTALL)
            text = re.sub(r"<style[^>]*>.*?</style>", " ", text, flags=re.IGNORECASE | re.DOTALL)
            text = re.sub(r"<[^>]+>", " ", text)
            return re.sub(r"\s+", " ", text).strip()

        # text-like
        try:
            return body.decode("utf-8", errors="replace")
        except Exception:
            return body.decode("utf-8", errors="ignore")
