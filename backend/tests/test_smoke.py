import time

from app.main import app


def wait_job(client, job_id: int, deadline: float = 30.0) -> dict:
    """Poll a background job until it finishes; returns the job payload."""
    start = time.monotonic()
    while time.monotonic() < start + deadline:
        job = client.get(f"/sources/jobs/{job_id}")
        assert job.status_code == 200, job.text
        payload = job.json()
        if payload["status"] != "running":
            return payload
        time.sleep(0.2)
    raise AssertionError(f"job {job_id} did not finish within {deadline}s")


def start_and_wait(client, source_id: int, kind: str = "sync", deadline: float = 30.0) -> dict:
    resp = client.post(f"/sources/{source_id}/{kind}")
    assert resp.status_code == 202, resp.text
    job = resp.json()
    assert job["status"] == "running"
    return wait_job(client, job["id"], deadline=deadline)


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

    job = start_and_wait(client, source.json()["id"])
    assert job["status"] == "done"
    assert job["result"]["items"] == 1
    assert job["result"]["entities"] >= 1

    # re-sync must dedupe
    job2 = start_and_wait(client, source.json()["id"])
    assert job2["result"]["items"] == 0

    entities = client.get("/entities").json()
    assert len(entities) >= 1
    assert {e["kind"] for e in entities} & {"decision", "action", "document"}

    # QA degrades gracefully without models
    qa = client.post("/qa", json={"question": "What did we decide?"})
    assert qa.status_code == 200
    assert qa.json()["answer"]


def test_delete_source_cascades_entities(client, tmp_path):
    """Deleting a source must remove its items, entities and chunks (NOT NULL FK bug:
    the ORM used to NULL entities.item_id instead of cascading, then the watcher's
    unschedule raised KeyError)."""
    from sqlalchemy import select

    from app.db import SessionLocal
    from app.models import Chunk, Entity, IngestedItem

    (tmp_path / "d.md").write_text(
        "We decided to delete this source. Action: clean up afterwards.",
        encoding="utf-8",
    )
    source = client.post(
        "/sources",
        json={"connector": "folder", "name": "doomed", "config": {"path": str(tmp_path)}},
    ).json()
    job = start_and_wait(client, source["id"])
    assert job["status"] == "done"
    assert job["result"]["entities"] >= 1

    with SessionLocal() as db:
        item = db.execute(select(IngestedItem).where(IngestedItem.source_id == source["id"])).scalar_one()
        entity_count = len(db.execute(select(Entity).where(Entity.item_id == item.id)).scalars().all())
        assert entity_count >= 1
        chunk_count = len(db.execute(select(Chunk).where(Chunk.item_id == item.id)).scalars().all())
        assert chunk_count >= 1

    resp = client.delete(f"/sources/{source['id']}")
    assert resp.status_code == 200, resp.text
    assert resp.json() == {"deleted": True}

    with SessionLocal() as db:
        assert db.get(IngestedItem, item.id) is None
        assert db.execute(select(Entity).where(Entity.item_id == item.id)).scalars().all() == []
        assert db.execute(select(Chunk).where(Chunk.item_id == item.id)).scalars().all() == []


def test_job_history_and_progress(client, tmp_path):
    (tmp_path / "a.md").write_text("We decided to ship v1 in June.", encoding="utf-8")
    (tmp_path / "b.md").write_text("Action: document the release process.", encoding="utf-8")
    source = client.post(
        "/sources",
        json={"connector": "folder", "name": "hist", "config": {"path": str(tmp_path)}},
    ).json()
    job = start_and_wait(client, source["id"])
    assert job["total"] == 2
    assert job["processed"] == 2

    history = client.get(f"/sources/jobs?source_id={source['id']}").json()
    assert [j["id"] for j in history] == [job["id"]]
    assert history[0]["kind"] == "sync"
    assert history[0]["status"] == "done"
    assert history[0]["result"]["entities"] >= 1


def test_reclassify_runs_as_job(client, tmp_path):
    (tmp_path / "r.md").write_text("We decided to adopt the new stack.", encoding="utf-8")
    source = client.post(
        "/sources",
        json={"connector": "folder", "name": "recl", "config": {"path": str(tmp_path)}},
    ).json()
    start_and_wait(client, source["id"])
    assert client.get("/entities").json()

    job = start_and_wait(client, source["id"], kind="reclassify")
    assert job["status"] == "done"
    assert job["kind"] == "reclassify"
    assert job["result"]["entities"] >= 1
    assert client.get("/entities").json()


def test_review_endpoints(client, tmp_path):
    (tmp_path / "a.md").write_text("We decided to ship v1 in June.", encoding="utf-8")
    source = client.post(
        "/sources",
        json={"connector": "folder", "name": "notes", "config": {"path": str(tmp_path)}},
    ).json()
    start_and_wait(client, source["id"])

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

    job = start_and_wait(client, source["id"])
    assert job["status"] == "failed"
    assert "does not exist" in (job["error"] or "")

    sources = client.get("/sources").json()
    failing = next(s for s in sources if s["id"] == source["id"])
    assert failing["error_count"] == 1
    assert "does not exist" in (failing["last_error"] or "")


def test_job_404(client):
    assert client.get("/sources/jobs/999999").status_code == 404


def test_cancel_running_job(client, tmp_path):
    """A long reclassify can be cancelled; the job ends 'cancelled' and the
    cancel endpoint rejects non-running jobs."""
    from app.db import SessionLocal
    from app.models import IngestedItem
    from sqlalchemy import select

    # a big doc so classification takes long enough to cancel mid-flight
    (tmp_path / "big.md").write_text(
        "We decided to ship the platform in June. " * 400, encoding="utf-8"
    )
    source = client.post(
        "/sources",
        json={"connector": "folder", "name": "cancel", "config": {"path": str(tmp_path)}},
    ).json()
    start_and_wait(client, source["id"])

    resp = client.post(f"/sources/{source['id']}/reclassify")
    assert resp.status_code == 202
    job_id = resp.json()["id"]

    cancel = client.post(f"/sources/jobs/{job_id}/cancel")
    assert cancel.status_code == 200, cancel.text
    assert cancel.json()["cancelled"] is True

    # job must reach a terminal state within the deadline
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        job = client.get(f"/sources/jobs/{job_id}").json()
        if job["status"] != "running":
            break
        time.sleep(0.2)
    assert job["status"] == "cancelled", job

    # cancelling a finished job → 409
    again = client.post(f"/sources/jobs/{job_id}/cancel")
    assert again.status_code == 409
    # unknown job → 404
    assert client.post("/sources/jobs/999999/cancel").status_code == 404


def test_orphaned_running_job_is_cancelled(client):
    """A job left 'running' by a dead process (no live task) is cancelled on
    JobManager init and via the cancel endpoint — it must not 409 forever."""
    from app.db import SessionLocal
    from app.jobs import JobManager
    from app.models import Job
    from app.pipeline import IngestionPipeline
    from app.app_settings import SettingsService
    from app.classifier import Classifier
    from app.config import settings
    from app.embedder import Embedder
    from app.secrets import SecretStore

    # create a fake running job row with no task behind it
    with SessionLocal() as db:
        fake = Job(source_id=1, kind="reclassify", status="running")
        db.add(fake)
        db.commit()
        job_id = fake.id

    svc = SettingsService(settings, SecretStore(settings.data_dir / "secrets.enc"))
    pipeline = IngestionPipeline(Classifier(svc), Embedder(svc), SecretStore(settings.data_dir / "secrets.enc"))
    jobs = JobManager(pipeline)  # init marks orphans cancelled
    with SessionLocal() as db:
        job = db.get(Job, job_id)
        assert job.status == "cancelled", "orphaned job must be marked cancelled on startup"

    # and a fresh orphan (created after init) is cancelled via the endpoint
    with SessionLocal() as db:
        fake2 = Job(source_id=1, kind="reclassify", status="running")
        db.add(fake2)
        db.commit()
        job2_id = fake2.id
    resp = client.post(f"/sources/jobs/{job2_id}/cancel")
    assert resp.status_code == 200, resp.text
    with SessionLocal() as db:
        job2 = db.get(Job, job2_id)
        assert job2.status == "cancelled"


def test_health_reports_components(client):
    health = client.get("/health").json()
    assert health["status"] == "ok"
    assert health["components"]["ollama"] in {"ok", "offline"}
    assert "tasks" in health["components"]
    assert "failing_sources" in health["components"]


def test_folder_watcher_picks_up_new_files(client, tmp_path):
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

    start_and_wait(client, source["id"])
    assert client.get("/entities").json()

    start_and_wait(client, source["id"], kind="reclassify")
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
    start_and_wait(client, source["id"])

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
    start_and_wait(client, source["id"])

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
