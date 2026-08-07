"""B3: entity dispute tracking.

Verifies the DoD: dispute an entity → counter increments, reason stored,
answers stop citing it (disputed excluded from Q&A retrieval by default).
"""

from tests.test_smoke import start_and_wait


def _mk_source(client, tmp_path, name: str, text: str) -> dict:
    (tmp_path / "d.md").write_text(text, encoding="utf-8")
    source = client.post(
        "/sources",
        json={"connector": "folder", "name": name, "config": {"path": str(tmp_path)}},
    ).json()
    job = start_and_wait(client, source["id"])
    assert job["status"] == "done"
    return source


def _entity_with_term(client, term: str) -> dict | None:
    for e in client.get("/entities").json():
        if term.lower() in (e.get("summary") or "").lower():
            return e
    return None


def test_dispute_records_audit_and_increments(client, tmp_path):
    _mk_source(
        client,
        tmp_path,
        "dispute-me",
        "We decided to adopt the azure-blue stack for checkout.",
    )
    entity = _entity_with_term(client, "azure-blue")
    assert entity is not None

    resp = client.post(
        f"/entities/{entity['id']}/dispute",
        json={"reason": "superseded by the teal decision", "user": "reviewer@corp"},
    )
    assert resp.status_code == 200, resp.text
    updated = resp.json()
    assert updated["status"] == "disputed"
    assert updated["dispute_count"] == 1

    # the audit trail is queryable
    history = client.get(f"/entities/{entity['id']}/disputes").json()
    assert len(history) == 1
    assert history[0]["reason"] == "superseded by the teal decision"
    assert history[0]["user"] == "reviewer@corp"

    # a second dispute increments the counter
    again = client.post(f"/entities/{entity['id']}/dispute", json={"reason": "double-checked"})
    assert again.status_code == 200
    assert again.json()["dispute_count"] == 2
    assert len(client.get(f"/entities/{entity['id']}/disputes").json()) == 2

    # unknown entity → 404
    assert client.post("/entities/999999/dispute", json={"reason": "x"}).status_code == 404
    assert client.get("/entities/999999/disputes").status_code == 404


def test_disputed_entity_stops_being_cited(client, tmp_path):
    """DoD: answers stop citing the entity once it is disputed."""
    _mk_source(
        client,
        tmp_path,
        "quasimoto",
        "We decided to paint the barn quasimoto purple for the launch.",
    )
    entity = _entity_with_term(client, "quasimoto")
    assert entity is not None

    # before the dispute the entity's own chunk is cited
    before = client.post("/qa", json={"question": "quasimoto paint"}).json()
    cited_ids = [c["entity_id"] for c in before["citations"]]
    assert entity["id"] in cited_ids, f"entity {entity['id']} must be cited before dispute: {before['citations']}"

    # dispute it (no reason required)
    resp = client.post(f"/entities/{entity['id']}/dispute", json={})
    assert resp.status_code == 200
    assert resp.json()["dispute_count"] == 1

    # after the dispute the entity's chunk is excluded from Q&A
    after = client.post("/qa", json={"question": "quasimoto paint"}).json()
    cited_after = [c for c in after["citations"] if c["entity_id"] == entity["id"]]
    assert not cited_after, f"disputed entity must not be cited: {after['citations']}"
    # the same doc's full-document chunk may still surface, but never via the entity
    assert all(c["entity_id"] != entity["id"] for c in after["citations"])
