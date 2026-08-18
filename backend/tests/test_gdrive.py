"""B28: GDrive connector — mocked Drive API (no real credentials)."""

import json
from unittest.mock import AsyncMock, MagicMock

import pytest

from app.connectors.gdrive import GDriveConnector


def _resp(json_data=None, text="", content=b"", status=200, headers=None):
    m = MagicMock()
    m.status_code = status
    m.headers = headers or {}
    m.json.return_value = json_data or {}
    m.text = text
    m.content = content
    m.raise_for_status = MagicMock()
    if status >= 400:
        m.raise_for_status.side_effect = Exception(f"HTTP {status}")
    return m


@pytest.mark.asyncio
async def test_gdrive_missing_config():
    from app.connectors.base import ConnectorError

    with pytest.raises(ConnectorError):
        GDriveConnector({})
    with pytest.raises(ConnectorError):
        GDriveConnector({"folder_id": "abc"})
    with pytest.raises(ConnectorError):
        GDriveConnector({"token": "tok"})


@pytest.mark.asyncio
async def test_gdrive_fetch_lists_and_exports(monkeypatch):
    # Drive list returns 2 files: one native doc, one pdf-like regular file
    list_payload = {
        "files": [
            {
                "id": "doc1",
                "name": "Planning notes",
                "mimeType": "application/vnd.google-apps.document",
                "modifiedTime": "2026-08-10T10:00:00.000Z",
            },
            {
                "id": "file2",
                "name": "spec.txt",
                "mimeType": "text/plain",
                "modifiedTime": "2026-08-11T11:00:00.000Z",
            },
        ]
    }

    # mock RetryClient as async context manager returning mocked get
    export_resp = _resp(text="Planning is about billing migration. Owner: Sarah.")
    media_resp = _resp(content=b"spec content about security transfers")

    async def mock_get(url, headers=None, params=None):
        if "export" in url:
            assert "doc1" in url
            return export_resp
        if params and params.get("alt") == "media":
            return media_resp
        # list call
        assert "drive/v3/files" in url
        assert params["q"].startswith("'folder123' in parents")
        return _resp(json_data=list_payload)

    mock_client = AsyncMock()
    mock_client.__aenter__.return_value = mock_client
    mock_client.__aexit__.return_value = False
    mock_client.get.side_effect = mock_get

    monkeypatch.setattr("app.connectors.gdrive.RetryClient", lambda timeout=60.0: mock_client)

    c = GDriveConnector({"folder_id": "folder123", "token": "tok123"})
    docs, cursor = await c.fetch("")
    assert len(docs) == 2
    titles = {d.title for d in docs}
    assert "Planning notes" in titles
    assert "spec.txt" in titles
    # cursor is newest modifiedTime
    assert "2026-08-11" in cursor

    # incremental: since_cursor filters via q
    captured_q = {}

    async def mock_get2(url, headers=None, params=None):
        if params and "q" in params:
            captured_q["q"] = params["q"]
        if params and params.get("alt") == "media":
            return media_resp
        if "export" in url:
            return export_resp
        return _resp(json_data={"files": []})

    mock_client.get.side_effect = mock_get2
    docs2, _ = await c.fetch(cursor)
    assert "modifiedTime >" in captured_q["q"]


@pytest.mark.asyncio
async def test_gdrive_auth_401(monkeypatch):
    from app.connectors.base import ConnectorError

    bad = _resp(status=401)
    mock_client = AsyncMock()
    mock_client.__aenter__.return_value = mock_client
    mock_client.__aexit__.return_value = False
    mock_client.get.return_value = bad
    monkeypatch.setattr("app.connectors.gdrive.RetryClient", lambda timeout=60.0: mock_client)

    c = GDriveConnector({"folder_id": "f", "token": "bad"})
    with pytest.raises(ConnectorError, match="401"):
        await c.fetch("")


@pytest.mark.asyncio
async def test_gdrive_end_to_end_via_pipeline(monkeypatch):
    """IngestionPipeline can sync a gdrive source (mocked fetch)."""
    from fastapi.testclient import TestClient

    # reuse test smoke's TestClient pattern — monkeypatch the connector fetch
    list_payload = {
        "files": [
            {
                "id": "g1",
                "name": "retro.md",
                "mimeType": "text/plain",
                "modifiedTime": "2026-08-12T00:00:00.000Z",
            }
        ]
    }
    media = _resp(content=b"retro meeting: decision about billing, owner Sarah")

    async def mock_get(url, headers=None, params=None):
        if params and params.get("alt") == "media":
            return media
        return _resp(json_data=list_payload)

    mock_client = AsyncMock()
    mock_client.__aenter__.return_value = mock_client
    mock_client.__aexit__.return_value = False
    mock_client.get.side_effect = mock_get
    monkeypatch.setattr("app.connectors.gdrive.RetryClient", lambda timeout=60.0: mock_client)

    from app.main import app

    with TestClient(app) as client:
        src = client.post("/sources", json={"connector": "gdrive", "name": "Drive Demo", "config": {"folder_id": "fid123", "token": "tok"}}).json()
        assert src["connector"] == "gdrive"
        # config masked
        cfg = client.get(f"/sources/{src['id']}/config").json()
        assert cfg["folder_id"] == "fid123"
        assert cfg["token"] == "***set***"

        job = client.post(f"/sources/{src['id']}/sync").json()
        # wait for job (uses same helper as test_smoke)
        import time

        for _ in range(30):
            gj = client.get(f"/sources/jobs/{job['id']}").json()
            if gj["status"] != "running":
                break
            time.sleep(0.2)
        assert gj["status"] == "done", gj
        ents = client.get("/entities").json()
        assert any("retro" in e["summary"].lower() or "billing" in e["summary"].lower() for e in ents)
