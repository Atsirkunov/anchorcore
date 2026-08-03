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

    # give the watchdog observer a beat to start watching before we create the file
    time.sleep(1)
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


def test_merge_cleans_references(client, tmp_path):
    """Merging an entity with relationship/merge rows must not 500 (NOT NULL FK bug)."""
    from app.db import SessionLocal
    from app.models import MergeAction, Relationship

    (tmp_path / "d0.md").write_text("We decided to adopt stack number zero.", encoding="utf-8")
    (tmp_path / "d1.md").write_text("We decided to adopt stack number one.", encoding="utf-8")
    source = client.post(
        "/sources",
        json={"connector": "folder", "name": "merge", "config": {"path": str(tmp_path)}},
    ).json()
    client.post(f"/sources/{source['id']}/sync")

    entities = client.get("/entities").json()
    assert len(entities) >= 2
    a, b = entities[0], entities[1]

    with SessionLocal() as db:
        action = MergeAction(entity_a_id=a["id"], entity_b_id=b["id"], status="proposed")
        db.add(action)
        db.add(Relationship(from_entity_id=a["id"], to_entity_id=b["id"], kind="related"))
        db.commit()
        proposal_id = action.id

    resp = client.post("/review/merge", json={"proposal_id": proposal_id, "decision": "merge"})
    assert resp.status_code == 200, resp.text
    assert resp.json()["merged"] is True


def test_full_document_chunking(client, tmp_path):
    """Long documents get section-aware chunks beyond entity summaries."""
    from sqlalchemy import select

    from app.db import SessionLocal
    from app.models import Chunk, Entity, IngestedItem

    paragraphs = [
        "We decided to launch the beta in June.",
        *[f"Paragraph number {i} discussing the feature set in detail." for i in range(1, 29)],
        "Action item: finish the remaining rollout before the launch.",
    ]
    (tmp_path / "big.md").write_text("\n\n".join(paragraphs), encoding="utf-8")

    source = client.post(
        "/sources",
        json={"connector": "folder", "name": "big", "config": {"path": str(tmp_path)}},
    ).json()
    client.post(f"/sources/{source['id']}/sync")

    with SessionLocal() as db:
        item = db.execute(select(IngestedItem).where(IngestedItem.source_id == source["id"])).scalar_one()
        assert item.text, "raw text must be stored"
        doc_chunks = db.execute(
            select(Chunk).where(Chunk.item_id == item.id, Chunk.entity_id.is_(None))
        ).scalars().all()
        assert len(doc_chunks) >= 2, "long doc must produce multiple full-doc chunks"
        assert any("Paragraph number 28" in c.content for c in doc_chunks), (
            "content beyond the first window must be chunked"
        )
        entities = db.execute(select(Entity).where(Entity.item_id == item.id)).scalars().all()
        assert entities, "classification still produces entities"

    # QA (keyword fallback, no models in tests) must return citations
    qa = client.post("/qa", json={"question": "What is discussed?"})
    assert qa.status_code == 200
    assert qa.json()["citations"]
