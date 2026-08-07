import logging

from app.redact import RedactingFormatter, redact


def test_redact_masks_secrets():
    assert "sk-abc1234567890123456789012" not in redact("key is sk-abc1234567890123456789012")
    assert "***REDACTED***" in redact("key is sk-abc1234567890123456789012")
    assert "Basic aGVsbG8td29ybGQtdGhpcy1pcy1hLXNlY3JldA==" not in redact(
        "Authorization: Basic aGVsbG8td29ybGQtdGhpcy1pcy1hLXNlY3JldA=="
    )
    assert "s3cr3t-tok3n" not in redact("jira token=s3cr3t-tok3n failed")
    assert "***REDACTED***" in redact("jira token=s3cr3t-tok3n failed")
    assert "password: hunter2" not in redact("login password: hunter2")


def test_redacting_formatter_masks_log_lines():
    formatter = RedactingFormatter("%(message)s")
    record = logging.LogRecord(
        "t", logging.ERROR, __file__, 1, "connector failed with sk-abcdefghijklmnopqrstuvwxyz123456", (), None
    )
    line = formatter.format(record)
    assert "sk-abcdefghijklmnopqrstuvwxyz123456" not in line
    assert "***REDACTED***" in line


def test_system_status(client):
    status = client.get("/system/status").json()
    assert status["version"]
    assert "data_dir" in status
    assert "tasks" in status
    assert status["ollama"]["reachable"] in {True, False}
    assert isinstance(status["pending_embeddings"], int)
    assert "answer" in status


def test_sync_failure_records_structured_event(client, tmp_path):
    source = client.post(
        "/sources",
        json={"connector": "folder", "name": "evt", "config": {"path": str(tmp_path / "nope")}},
    ).json()
    resp = client.post(f"/sources/{source['id']}/sync")
    assert resp.status_code == 202
    job = resp.json()
    deadline = 15.0
    import time

    start = time.monotonic()
    while time.monotonic() < start + deadline:
        job = client.get(f"/sources/jobs/{job['id']}").json()
        if job["status"] != "running":
            break
        time.sleep(0.2)
    assert job["status"] == "failed"

    events = client.get("/system/errors?component=pipeline").json()
    mine = [e for e in events if e["source_id"] == source["id"]]
    assert mine, "no pipeline event recorded for the failed sync"
    assert "sync failed for source" in mine[0]["message"]
    assert "does not exist" in mine[0]["detail"]


def test_qa_failure_records_warning(client):
    qa = client.post("/qa", json={"question": "What was decided?"})
    assert qa.status_code == 200
    events = client.get("/system/errors?component=qa").json()
    assert any("retrieval degraded" in e["message"] or "answer generation" in e["message"] for e in events)


def test_get_config_redacts_secrets(client):
    source = client.post(
        "/sources",
        json={
            "connector": "jira",
            "name": "j",
            "config": {"base_url": "https://x.atlassian.net", "email": "a@b.c", "token": "supersecret", "project": "PM"},
        },
    ).json()
    config = client.get(f"/sources/{source['id']}/config").json()
    assert config["token"] == "***set***"
    assert "supersecret" not in str(config)


def test_logs_list_and_download(client):
    files = client.get("/system/logs").json()
    assert any(f["name"] == "anchorcore.log" for f in files), files

    resp = client.get("/system/logs/anchorcore.log")
    assert resp.status_code == 200
    assert "text/plain" in resp.headers["content-type"]

    assert client.get("/system/logs/anchorcore.log.5").status_code == 404
    assert client.get("/system/logs/..%2F..%2Fetc%2Fpasswd").status_code == 404
    assert client.get("/system/logs/evil.txt").status_code == 404


def test_health_reports_pending_embedding_count(client):
    health = client.get("/health").json()
    assert isinstance(health["components"]["pending_embeddings"], int)


def _onboarding_fn():
    """The route's underlying function, so tests can call it against any DB."""
    from app.main import app

    route = next(r for r in app.routes if getattr(r, "path", "") == "/system/onboarding")
    return route.endpoint


def test_onboarding_fresh_install_needs_wizard(tmp_path):
    """B8: on a genuinely empty database the wizard must trigger. Verifies the
    real empty state with its own fresh DB (the shared test DB is never empty,
    so the HTTP fixture can't exercise this branch)."""
    from sqlalchemy import create_engine
    from sqlalchemy.orm import Session

    from app.db import Base

    engine = create_engine(f"sqlite:///{tmp_path / 'fresh.db'}")
    Base.metadata.create_all(engine)
    import asyncio

    with Session(engine) as db:
        state = asyncio.run(_onboarding_fn()(db))
    assert state["needs_wizard"] is True
    assert state["sources_count"] == 0
    assert "reachable" in state["ollama"]
    assert "missing_models" in state["ollama"]
    assert state["answer_provider"] in {"ollama", "configured", "missing"}


def test_onboarding_state(client, tmp_path):
    """B8: the wizard state reflects newly connected sources (assert the delta
    on the shared test DB — never global emptiness) and keeps its shape."""
    before = client.get("/system/onboarding").json()
    assert set(before) == {"needs_wizard", "sources_count", "ollama", "answer_provider", "sample"}
    assert isinstance(before["sources_count"], int)
    assert isinstance(before["ollama"]["missing_models"], list)
    assert "sample" in before
    assert isinstance(before["sample"]["available"], bool)
    if before["sample"]["available"]:
        assert before["sample"]["path"]

    (tmp_path / "d.md").write_text("We decided to keep going.", encoding="utf-8")
    client.post(
        "/sources",
        json={"connector": "folder", "name": "wiz", "config": {"path": str(tmp_path)}},
    ).json()

    after = client.get("/system/onboarding").json()
    assert after["sources_count"] >= before["sources_count"] + 1
