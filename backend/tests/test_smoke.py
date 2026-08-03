import pytest
from fastapi.testclient import TestClient

from app.main import app


@pytest.fixture()
def client(tmp_path, monkeypatch):
    monkeypatch.setenv("ANCHOR_DATABASE_URL", f"sqlite:///{tmp_path / 'test.db'}")
    monkeypatch.setenv("ANCHOR_OLLAMA_BASE_URL", "http://localhost:1")
    monkeypatch.setenv("ANCHOR_CLASSIFIER_TIMEOUT", "1.0")
    monkeypatch.setenv("ANCHOR_ANSWER_BASE_URL", "http://localhost:1")
    with TestClient(app) as c:
        yield c


def test_health(client):
    assert client.get("/health").status_code == 200


def test_folder_ingest_flow(client, tmp_path):
    (tmp_path / "notes.md").write_text(
        "We decided to postpone the launch. Action: Sarah owns the migration. Status: green.",
        encoding="utf-8",
    )
    source = client.post(
        "/sources",
        json={"connector": "folder", "name": "notes", "config": {"path": str(tmp_path)}},
    )
    assert source.status_code == 201

    sync = client.post(f"/sources/{source.json()['id']}/sync")
    assert sync.status_code == 200
    assert sync.json()["items"] == 1
    assert sync.json()["entities"] >= 1

    # re-sync must dedupe
    sync2 = client.post(f"/sources/{source.json()['id']}/sync")
    assert sync2.json()["items"] == 0

    entities = client.get("/entities").json()
    assert len(entities) >= 1
    assert {e["kind"] for e in entities} & {"decision", "action", "document"}

    # QA degrades gracefully without models
    qa = client.post("/qa", json={"question": "What did we decide?"})
    assert qa.status_code == 200
    assert qa.json()["answer"]


def test_review_endpoints(client, tmp_path):
    (tmp_path / "a.md").write_text("We decided to ship v1 in June.", encoding="utf-8")
    source = client.post(
        "/sources",
        json={"connector": "folder", "name": "notes", "config": {"path": str(tmp_path)}},
    ).json()
    client.post(f"/sources/{source['id']}/sync")

    low = client.get("/review/low-confidence")
    assert low.status_code == 200

    entity = client.get("/entities").json()[0]
    patch = client.patch(f"/entities/{entity['id']}", json={"status": "verified"})
    assert patch.status_code == 200
    assert patch.json()["status"] == "verified"
