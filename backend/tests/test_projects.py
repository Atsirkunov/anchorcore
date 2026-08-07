"""B15: scoped search via projects.

Verifies the DoD: connect 3+ sources, create two projects with overlapping
sources, and confirm the same question returns project-scoped results.
"""

from sqlalchemy import select

from app.db import SessionLocal
from app.models import Project


def _mk_source(client, tmp_path, folder: str, name: str) -> dict:
    d = tmp_path / folder
    d.mkdir(exist_ok=True)
    (d / "doc.md").write_text(
        f"# {name}\nDecision: the {name.lower()} team ships the DVCA movement rules.\n",
        encoding="utf-8",
    )
    source = client.post(
        "/sources",
        json={"connector": "folder", "name": name, "config": {"path": str(d)}},
    ).json()
    from tests.test_smoke import start_and_wait

    job = start_and_wait(client, source["id"])
    assert job["status"] == "done"
    return source


def test_project_crud(client):
    resp = client.post("/projects", json={"name": "payments"})
    assert resp.status_code == 201, resp.text
    pid = resp.json()["id"]

    resp = client.get(f"/projects/{pid}")
    assert resp.status_code == 200
    assert resp.json()["name"] == "payments"

    resp = client.patch(f"/projects/{pid}", json={"name": "payments-core", "is_default": True})
    assert resp.status_code == 200, resp.text
    assert resp.json()["name"] == "payments-core"
    assert resp.json()["is_default"] is True

    resp = client.get("/projects/default")
    assert resp.status_code == 200
    assert resp.json()["id"] == pid

    resp = client.delete(f"/projects/{pid}")
    assert resp.status_code == 200
    assert client.get("/projects").json() == []


def test_project_rejects_unknown_source(client):
    resp = client.post("/projects", json={"name": "bad", "source_ids": [99999]})
    assert resp.status_code == 422, resp.text


def test_default_project_is_singleton(client):
    a = client.post("/projects", json={"name": "A"}).json()
    b = client.post("/projects", json={"name": "B"}).json()
    client.patch(f"/projects/{a['id']}", json={"is_default": True})
    client.patch(f"/projects/{b['id']}", json={"is_default": True})
    projects = client.get("/projects").json()
    defaults = [p for p in projects if p["is_default"]]
    assert len(defaults) == 1 and defaults[0]["id"] == b["id"]


def test_project_scopes_qa_results(client, tmp_path):
    """DoD: two projects with overlapping sources — the same question returns
    project-scoped results (only the project's sources are cited)."""
    alpha = _mk_source(client, tmp_path, "alpha", "Alpha")
    beta = _mk_source(client, tmp_path, "beta", "Beta")
    gamma = _mk_source(client, tmp_path, "gamma", "Gamma")

    # Project P1 = {alpha, beta}; Project P2 = {beta, gamma} (overlap: beta)
    p1 = client.post("/projects", json={"name": "P1", "source_ids": [alpha["id"], beta["id"]]}).json()
    p2 = client.post("/projects", json={"name": "P2", "source_ids": [beta["id"], gamma["id"]]}).json()

    # unscoped query sees all three sources' content
    unscoped = client.post("/qa", json={"question": "DVCA movement rules"}).json()
    all_text = " ".join(c["source_ref"] for c in unscoped["citations"])
    assert "/alpha/" in all_text and "/beta/" in all_text, "unscoped query must span sources"

    # scoped to P1 → only alpha + beta sources cited
    scoped1 = client.post(
        "/qa", json={"question": "DVCA movement rules", "project_id": p1["id"]}
    ).json()
    refs1 = " ".join(c["source_ref"] for c in scoped1["citations"])
    assert "/alpha/" in refs1 and "/beta/" in refs1, f"P1 must include its sources: {refs1}"
    assert "/gamma/" not in refs1, f"P1 must not include gamma: {refs1}"

    # scoped to P2 → gamma + beta, never alpha
    scoped2 = client.post(
        "/qa", json={"question": "DVCA movement rules", "project_id": p2["id"]}
    ).json()
    refs2 = " ".join(c["source_ref"] for c in scoped2["citations"])
    assert "/gamma/" in refs2, f"P2 must include gamma: {refs2}"
    assert "/alpha/" not in refs2, f"P2 must not include alpha: {refs2}"


def test_project_sources_relationship(client, tmp_path):
    a = _mk_source(client, tmp_path, "one", "One")
    b = _mk_source(client, tmp_path, "two", "Two")
    p = client.post("/projects", json={"name": "pair", "source_ids": [a["id"], b["id"]]}).json()
    assert set(p["source_ids"]) == {a["id"], b["id"]}

    with SessionLocal() as db:
        proj = db.get(Project, p["id"])
        assert sorted(s.id for s in proj.sources) == sorted([a["id"], b["id"]])
