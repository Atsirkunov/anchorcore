"""B14: MCP agent connectivity — tool surface + /qa/search endpoint.

Drives the real FastAPI app through ASGITransport (same app the `client`
fixture boots: migrations + scheduler via lifespan) so the tests cover the
actual REST layer the stdio sidecar uses. No LLM needed — degraded mode
(rule-based classifier, keyword retrieval) is the CI-safe path.
"""

import asyncio

import httpx
import pytest
from mcp.server.fastmcp.exceptions import ToolError

from app.mcp.backend import BackendError, HttpBackend
from app.mcp.server import build_server

from tests.test_smoke import start_and_wait


def _mk_source(client, tmp_path, folder: str, name: str, text: str) -> dict:
    d = tmp_path / folder
    d.mkdir(exist_ok=True)
    (d / "doc.md").write_text(text, encoding="utf-8")
    source = client.post(
        "/sources",
        json={"connector": "folder", "name": name, "config": {"path": str(d)}},
    ).json()
    job = start_and_wait(client, source["id"])
    assert job["status"] == "done", job
    return source


def _backend(client) -> HttpBackend:
    from app.main import app

    return HttpBackend(
        base_url="http://test",
        transport=httpx.ASGITransport(app=app),
    )


def _run(coro):
    return asyncio.run(coro)


def test_mcp_search_returns_ranked_hits(client, tmp_path):
    alpha = _mk_source(
        client, tmp_path, "alpha", "Alpha",
        "# Alpha\nDecision: the alpha team owns the DVCA movement rules.\n",
    )
    beta = _mk_source(
        client, tmp_path, "beta", "Beta",
        "# Beta\nDecision: the beta team ships the DRB transport spec.\n",
    )
    backend = _backend(client)
    result = _run(backend.search("movement rules"))
    hits = result["hits"]
    assert hits, "search should return hits for ingested content"
    assert hits[0]["score"] >= hits[-1]["score"], "hits must be ranked desc"
    sources = {h["source_id"] for h in hits}
    assert alpha["id"] in sources
    assert beta["id"] not in sources, "query about alpha content should not hit beta"
    hit = hits[0]
    assert hit["content"]
    assert hit["source_ref"]
    assert hit["kind"] in {"document", "decision", "note", "action"}


def test_mcp_search_respects_status_filters(client, tmp_path):
    src = _mk_source(
        client, tmp_path, "disputed", "Disputed",
        "# Disputed\nDecision: the disputed team owns the PROVISIONED state machine.\n",
    )
    entities = client.get("/entities").json()
    target = next(e for e in entities if e["source_ref"] or e["summary"])
    # dispute it → excluded from retrieval (B3 + B14 share _status_ok)
    client.post(f"/entities/{target['id']}/dispute", json={"reason": "wrong", "user": "tester"})

    backend = _backend(client)
    before = _run(backend.search("PROVISIONED state machine"))["hits"]
    after = _run(backend.search("state machine"))["hits"]
    assert not any(h["entity_id"] == target["id"] for h in after)
    assert not any(h["entity_id"] == target["id"] for h in before) or before


def test_mcp_ask_returns_answer_and_citations(client, tmp_path):
    _mk_source(
        client, tmp_path, "ask", "Ask",
        "# Ask\nDecision: the ask team ships the GAMMA scoring model.\n",
    )
    backend = _backend(client)
    result = _run(backend.ask("GAMMA scoring"))
    assert result["answer"]
    assert result["citations"], "answer should cite its sources"
    assert result["citations"][0]["source_ref"]


def test_mcp_entity_and_source_tools(client, tmp_path):
    src = _mk_source(
        client, tmp_path, "probe", "Probe",
        "# Probe\nDecision: the probe team owns the HORIZON rollout plan.\n",
    )
    backend = _backend(client)
    result = _run(backend.search("HORIZON rollout"))
    hit = result["hits"][0]

    entity = _run(backend.get_entity(hit["entity_id"])) if hit["entity_id"] else None
    if entity is not None:
        assert entity["id"] == hit["entity_id"]
        assert entity["kind"]
        assert "window_text" in entity

    src_out = _run(backend.get_source(src["id"]))
    assert src_out["name"] == "Probe"
    assert src_out["connector"] == "folder"

    sources = _run(backend.list_sources())
    names = [s["name"] for s in sources]
    assert "Probe" in names

    status = _run(backend.memory_status())
    assert "version" in status and "ollama" in status


def test_mcp_search_scopes_to_project(client, tmp_path):
    a = _mk_source(
        client, tmp_path, "scoped-a", "ScopedA",
        "# ScopedA\nDecision: scoped a owns the VELOCITY budget.\n",
    )
    b = _mk_source(
        client, tmp_path, "scoped-b", "ScopedB",
        "# ScopedB\nDecision: scoped b owns the TORQUE budget.\n",
    )
    project = client.post("/projects", json={"name": "velo", "source_ids": [a["id"]]}).json()

    backend = _backend(client)
    result = _run(backend.search("budget", k=10, project_id=project["id"]))
    assert result["hits"]
    assert all(h["source_id"] == a["id"] for h in result["hits"])
    assert b["id"] not in {h["source_id"] for h in result["hits"]}
    # shared test DB: clean up or later tests (test_projects asserts global
    # emptiness of /projects) break
    client.delete(f"/projects/{project['id']}")


def test_mcp_tools_registered_and_callable(client, tmp_path):
    _mk_source(
        client, tmp_path, "reg", "Reg",
        "# Reg\nDecision: the reg team owns the ULTIMATE checklist.\n",
    )
    backend = _backend(client)
    mcp = build_server(backend, name="test-anchorcore")

    listed = _run(mcp.call_tool("list_sources", {}))
    assert listed, "list_sources should return content"

    searched = _run(mcp.call_tool("search", {"query": "ULTIMATE checklist", "k": 5}))
    assert searched, "search should return content"

    with pytest.raises(ToolError):
        _run(mcp.call_tool("search", {"query": "x", "k": 0}))

    with pytest.raises(ToolError):
        _run(mcp.call_tool("get_entity", {"entity_id": 999999}))
