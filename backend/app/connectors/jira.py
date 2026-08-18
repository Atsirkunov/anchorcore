import logging
from datetime import datetime, timezone

from ..http import RetryClient
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

    def _jql(self, since_cursor: str = "") -> str:
        # Support single or comma-separated multiple projects: "PM,TEST" -> project in ("PM","TEST")
        raw = (self.project or "").strip()
        parts = [p.strip() for p in raw.split(",") if p.strip()]
        if len(parts) > 1:
            quoted = ",".join(f'"{p}"' for p in parts)
            base = f"project in ({quoted})"
        elif parts:
            base = f'project = "{parts[0]}"'
        else:
            base = f'project = "{self.project}"'
        if since_cursor:
            return f'{base} AND updated > "{since_cursor}" ORDER BY updated ASC'
        return f"{base} ORDER BY updated ASC"

    async def fetch(self, since_cursor: str = "") -> tuple[list[IngestionDoc], str]:
        jql = self._jql(since_cursor)

        docs: list[IngestionDoc] = []
        next_cursor = since_cursor
        next_page_token: str | None = None
        start_at = 0
        use_new_endpoint = True
        async with RetryClient(timeout=60.0) as client:
            while True:
                if use_new_endpoint:
                    params: dict[str, str | int] = {
                        "jql": jql,
                        "maxResults": 50,
                        "fields": "summary,description,comment,updated,creator,assignee,status",
                    }
                    if next_page_token:
                        params["nextPageToken"] = next_page_token
                    # Atlassian deprecated GET /rest/api/3/search (now 410 Gone) -> use /search/jql
                    # https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issue-search/#api-rest-api-3-search-jql-get
                    resp = await client.get(
                        f"{self.base_url}/rest/api/3/search/jql",
                        headers=self._auth(),
                        params=params,
                    )
                    if resp.status_code in (404, 410):
                        use_new_endpoint = False
                        # retry same loop iteration with old endpoint
                        continue
                else:
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

                if use_new_endpoint:
                    if data.get("isLast"):
                        break
                    next_page_token = data.get("nextPageToken")
                    if not next_page_token:
                        break
                else:
                    if start_at + len(data.get("issues", [])) >= data.get("total", 0):
                        break
                    start_at += 50

        return docs, next_cursor


async def list_jira_projects(base_url: str, email: str, token: str) -> list[dict]:
    """Query available Jira projects for the checkbox picker.
    Uses GET /rest/api/3/project/search (paginated, Cloud) with fallback to GET /rest/api/3/project (Server).
    Returns [{key, name}].
    """
    import base64

    base_url = base_url.rstrip("/")
    raw = f"{email}:{token}".encode()
    headers = {"Authorization": f"Basic {base64.b64encode(raw).decode()}"}
    projects: list[dict] = []
    next_page_token: str | None = None
    async with RetryClient(timeout=30.0) as client:
        # Try new Cloud endpoint first
        while True:
            params: dict[str, str | int] = {"maxResults": 50}
            if next_page_token:
                params["nextPageToken"] = next_page_token
            resp = await client.get(
                f"{base_url}/rest/api/3/project/search",
                headers=headers,
                params=params,
            )
            if resp.status_code in (404, 410):
                break  # fallback to legacy
            resp.raise_for_status()
            data = resp.json()
            for p in data.get("values", []):
                if p.get("key") and p.get("name"):
                    projects.append({"key": p["key"], "name": p["name"]})
            if data.get("isLast"):
                break
            next_page_token = data.get("nextPageToken")
            if not next_page_token:
                break
        if not projects:
            # Legacy fallback: GET /rest/api/3/project returns array
            resp = await client.get(f"{base_url}/rest/api/3/project", headers=headers)
            resp.raise_for_status()
            data = resp.json()
            if isinstance(data, list):
                for p in data:
                    if p.get("key") and p.get("name"):
                        projects.append({"key": p["key"], "name": p["name"]})
    return projects
