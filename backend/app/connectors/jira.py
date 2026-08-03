import logging
from datetime import datetime, timezone
from urllib.parse import quote

import httpx

from .base import BaseConnector, ConnectorError, IngestionDoc

logger = logging.getLogger(__name__)


class JiraConnector(BaseConnector):
    connector_type = "jira"

    def __init__(self, config: dict):
        super().__init__(config)
        self.base_url = (config.get("base_url") or "").rstrip("/")
        self.email = config.get("email") or ""
        self.token = config.get("token") or ""
        self.project = config.get("project") or ""
        if not self.base_url or not self.email or not self.token:
            raise ConnectorError("jira connector requires base_url, email, token")
        if not self.project:
            raise ConnectorError("jira connector requires project")

    def _auth(self) -> dict[str, str]:
        import base64

        raw = f"{self.email}:{self.token}".encode()
        return {"Authorization": f"Basic {base64.b64encode(raw).decode()}"}

    async def fetch(self, since_cursor: str = "") -> tuple[list[IngestionDoc], str]:
        jql = f'project = {quote(self.project)} ORDER BY updated ASC'
        if since_cursor:
            jql = f'project = {quote(self.project)} AND updated > "{since_cursor}" ORDER BY updated ASC'

        docs: list[IngestionDoc] = []
        next_cursor = since_cursor
        start_at = 0
        async with httpx.AsyncClient(timeout=60.0) as client:
            while True:
                resp = await client.get(
                    f"{self.base_url}/rest/api/3/search",
                    headers=self._auth(),
                    params={
                        "jql": jql,
                        "startAt": start_at,
                        "maxResults": 50,
                        "fields": "summary,description,comment,updated,creator,assignee,status",
                    },
                )
                resp.raise_for_status()
                data = resp.json()

                for issue in data.get("issues", []):
                    fields = issue.get("fields", {})
                    comments = [
                        (c.get("body") or "")
                        for c in (fields.get("comment") or {}).get("comments", [])
                    ]
                    author = ""
                    creator = fields.get("creator") or {}
                    assignee = fields.get("assignee") or {}
                    if creator:
                        author = creator.get("displayName") or creator.get("emailAddress") or ""
                    text_parts = [
                        fields.get("summary") or "",
                        f"Status: {(fields.get('status') or {}).get('name', '')}",
                        f"Assignee: {assignee.get('displayName', '') if assignee else ''}",
                        fields.get("description") or "",
                        *comments,
                    ]
                    updated_raw = fields.get("updated") or ""
                    updated = None
                    if updated_raw:
                        try:
                            updated = datetime.fromisoformat(updated_raw.replace("Z", "+00:00"))
                        except ValueError:
                            updated = None
                    docs.append(
                        IngestionDoc(
                            external_id=issue.get("key") or "",
                            title=fields.get("summary") or issue.get("key") or "",
                            text="\n\n".join(part for part in text_parts if part),
                            author=author,
                            updated_at=updated,
                            source_ref=f"Jira:{issue.get('key')}",
                        )
                    )
                    if updated and (not next_cursor or updated.isoformat() > next_cursor):
                        next_cursor = updated.isoformat()

                if start_at + len(data.get("issues", [])) >= data.get("total", 0):
                    break
                start_at += 50

        return docs, next_cursor
