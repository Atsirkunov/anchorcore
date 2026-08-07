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


def test_onboarding_state(client, tmp_path):
    """B8: the wizard trigger — shape is stable and the source count reflects
    newly connected sources (the shared test DB is never globally empty, so we
    assert the delta, not absolute emptiness)."""
    before = client.get("/system/onboarding").json()
    assert "needs_wizard" in before
    assert "sources_count" in before
    assert "reachable" in before["ollama"]
    assert "missing_models" in before["ollama"]
    assert before["answer_provider"] in {"ollama", "configured", "missing"}
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
    # needs_wizard is defined as "no sources at all" — once a source exists it
    # must be false
    assert after["needs_wizard"] is (after["sources_count"] == 0)
