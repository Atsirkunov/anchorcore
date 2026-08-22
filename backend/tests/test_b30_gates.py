"""B30 (full): answer-provider gate + public-only sharing scope.

Covers:
- /qa/public answers ONLY from `public` sources (share/MCP scope)
- sensitive/pii source content is blocked from unconfirmed cloud answers
- PII-flagged chunks are blocked from unconfirmed cloud answers even when the
  source label is internal
- the gate records a system_event and returns an explicit refusal when nothing
  survives the filter
"""

import os


def _mk_source(client, tmp_path, name, text, label="internal"):
    (tmp_path / f"{name}.md").write_text(text, encoding="utf-8")
    return client.post(
        "/sources",
        json={
            "connector": "folder",
            "name": name,
            "label": label,
            "config": {"path": str(tmp_path)},
        },
    ).json()


def _with_cloud_answer(monkeypatch):
    """Point the answer provider at a cloud URL (untrusted without the flag).

    The shared settings_svc is read by BOTH the trust probe
    (cloud_answer_trusted) and AnswerEngine._generate — patch its `get` so the
    effective answer_base_url is a cloud endpoint."""
    from app.main import settings_svc

    original_get = settings_svc.get

    def get(self, key, db=None):
        if key == "answer_base_url":
            return "https://api.openai.com/v1"
        if key == "answer_api_key":
            return "sk-test"
        return original_get(key, db=db)

    monkeypatch.setattr(settings_svc, "get", get.__get__(settings_svc, type(settings_svc)))
    return settings_svc


def test_public_ask_excludes_internal_sources(client, tmp_path):
    """B30 DoD: /qa/public only answers from public-labelled sources."""
    from tests.test_smoke import start_and_wait

    pub = _mk_source(
        client,
        tmp_path,
        "pub",
        "Decision: the public release date is January 5th for everyone.",
        label="public",
    )
    sec = _mk_source(
        client,
        tmp_path,
        "sec",
        "Decision: the internal restructuring is confidential to staff only.",
        label="internal",
    )
    start_and_wait(client, pub["id"])
    start_and_wait(client, sec["id"])

    resp = client.post("/qa/public", json={"question": "What was decided about release dates?"})
    assert resp.status_code == 200, resp.text
    refs = [c["source_ref"] for c in resp.json()["citations"]]
    assert any("pub" in r for r in refs), refs
    assert all("sec" not in r for r in refs), "internal source must not appear in public answers"


def test_public_ask_needs_a_public_source(client, tmp_path):
    """B30 DoD: /qa/public never leaks internal content — even when the only
    matching source is internal, its citations must not appear."""
    from tests.test_smoke import start_and_wait

    src = _mk_source(
        client,
        tmp_path,
        "only-internal",
        "Decision: the salary cap for executives is confidential.",
        label="internal",
    )
    start_and_wait(client, src["id"])

    resp = client.post("/qa/public", json={"question": "What is the salary cap?"})
    assert resp.status_code == 200, resp.text
    refs = [c["source_ref"] for c in resp.json()["citations"]]
    assert all("only-internal" not in r for r in refs), "internal source must not leak into public answers"


def test_sensitive_source_blocked_from_cloud_answer(client, tmp_path, monkeypatch):
    """B30 DoD: a sensitive-labelled source is excluded from an unconfirmed
    cloud answer — the response either refuses or answers from remaining hits."""
    import pytest

    if __import__("os").environ.get("ANCHOR_TEST_RUST_URL"):
        pytest.skip("Monkeypatching Python settings_svc not visible to Rust server")
    from tests.test_smoke import start_and_wait

    sens = _mk_source(
        client,
        tmp_path,
        "sens",
        "Decision: the HR complaint details are highly sensitive.",
        label="sensitive",
    )
    start_and_wait(client, sens["id"])

    _with_cloud_answer(monkeypatch)

    resp = client.post("/qa", json={"question": "What is in the HR complaint?"})
    assert resp.status_code == 200, resp.text
    body = resp.json()
    # either fully refused (no non-sensitive evidence) or only non-sensitive hits
    assert all(c["source_ref"].startswith("sens") is False for c in body["citations"]), body["citations"]


def test_pii_flagged_chunk_blocked_from_cloud_answer(client, tmp_path, monkeypatch):
    """B30: an internal source whose chunk is PII-flagged is blocked from the
    cloud answer path even though the source label is internal."""
    import pytest

    if __import__("os").environ.get("ANCHOR_TEST_RUST_URL"):
        pytest.skip("Monkeypatching Python settings_svc not visible to Rust server")
    from tests.test_smoke import start_and_wait

    src = _mk_source(
        client,
        tmp_path,
        "mixed",
        "Decision: contact joe.bloggs@example.com for the report; card 4111-1111-1111-1111.",
        label="internal",
    )
    start_and_wait(client, src["id"])

    # ensure the chunk is PII-flagged
    review = client.get("/pii/review").json()
    flagged = [r for r in review if "joe.bloggs@example.com" in r["content"]]
    assert flagged, "expected the scan to flag the email/card chunk"
    assert flagged[0]["is_pii"] is True

    _with_cloud_answer(monkeypatch)

    resp = client.post("/qa", json={"question": "Who should I contact for the report?"})
    assert resp.status_code == 200, resp.text
    body = resp.json()
    assert all("joe.bloggs@example.com" not in c["snippet"] for c in body["citations"]), (
        "PII chunk must not reach the cloud answer"
    )


def test_cloud_answer_trust_flag_allows_sensitive(client, tmp_path, monkeypatch):
    """B30: with ANCHOR_CLOUD_TRUST=1 the cloud answer provider is trusted and
    sensitive content is allowed through."""
    import pytest

    if os.environ.get("ANCHOR_TEST_RUST_URL"):
        pytest.skip("Monkeypatching Python settings_svc not visible to Rust server")
    from tests.test_smoke import start_and_wait

    sens = _mk_source(
        client,
        tmp_path,
        "trusted-sens",
        "Decision: the client's email is joe.bloggs@example.com.",
        label="sensitive",
    )
    start_and_wait(client, sens["id"])

    _with_cloud_answer(monkeypatch)
    os.environ["ANCHOR_CLOUD_TRUST"] = "1"
    try:
        resp = client.post("/qa", json={"question": "What email was mentioned?"})
        assert resp.status_code == 200, resp.text
        assert resp.json()["citations"], "trusted cloud provider may cite sensitive sources"
    finally:
        os.environ.pop("ANCHOR_CLOUD_TRUST", None)


def test_answer_gate_records_system_event(client, tmp_path, monkeypatch):
    """B30 audit: a blocked cloud answer lands a system_event."""
    import pytest

    if __import__("os").environ.get("ANCHOR_TEST_RUST_URL"):
        pytest.skip("Monkeypatching Python settings_svc not visible to Rust server")
    from tests.test_smoke import start_and_wait

    sens = _mk_source(
        client,
        tmp_path,
        "audit",
        "Decision: the CEO's phone is 555-123-4567.",
        label="pii",
    )
    start_and_wait(client, sens["id"])

    _with_cloud_answer(monkeypatch)

    client.post("/qa", json={"question": "What is the CEO's phone number?"})
    events = client.get("/system/errors", params={"component": "qa", "limit": 50}).json()
    assert any("answer gate" in e["message"] for e in events), events
