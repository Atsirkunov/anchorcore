from app.main import app


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


def test_sync_error_tracking(client, tmp_path):
    source = client.post(
        "/sources",
        json={"connector": "folder", "name": "bad", "config": {"path": str(tmp_path / "nope")}},
    ).json()

    sync = client.post(f"/sources/{source['id']}/sync")
    assert sync.status_code == 400

    sources = client.get("/sources").json()
    failing = next(s for s in sources if s["id"] == source["id"])
    assert failing["error_count"] == 1
    assert "does not exist" in (failing["last_error"] or "")


def test_health_reports_components(client):
    health = client.get("/health").json()
    assert health["status"] == "ok"
    assert health["components"]["ollama"] in {"ok", "offline"}
    assert "tasks" in health["components"]
    assert "failing_sources" in health["components"]


def test_folder_watcher_picks_up_new_files(client, tmp_path):
    import time

    source = client.post(
        "/sources",
        json={"connector": "folder", "name": "watched", "config": {"path": str(tmp_path)}},
    ).json()

    (tmp_path / "new.md").write_text("We decided to ship the watcher.", encoding="utf-8")

    deadline = time.monotonic() + 20
    found = False
    while time.monotonic() < deadline:
        time.sleep(1)
        entities = client.get("/entities").json()
        if any("watcher" in e["summary"] for e in entities):
            found = True
            break
    assert found, "watcher did not ingest the new file within 20s"
    assert source["id"] is not None


def test_reclassify_rebuilds_entities(client, tmp_path):
    (tmp_path / "r.md").write_text("We decided to adopt the new stack.", encoding="utf-8")
    source = client.post(
        "/sources",
        json={"connector": "folder", "name": "recl", "config": {"path": str(tmp_path)}},
    ).json()

    sync = client.post(f"/sources/{source['id']}/sync")
    assert sync.status_code == 200
    assert client.get("/entities").json()

    result = client.post(f"/sources/{source['id']}/reclassify")
    assert result.status_code == 200
    assert result.json()["entities"] >= 1
    assert client.get("/entities").json()
