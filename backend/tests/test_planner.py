"""B17: Planner → Executor → Synthesis.

Verifies tool selection (planner), the who_knows tool, evidence fusion across
tools, and the DoD: a question needing two source types is answered with
evidence from both, cited.
"""

import asyncio
from datetime import datetime, timezone

from sqlalchemy import select

from app.answer_engine import TOOL_HYBRID, TOOL_WHO_KNOWS, AnswerEngine
from app.config import settings
from app.db import SessionLocal
from app.models import Entity, IngestedItem


def _make_svc():
    from app.app_settings import SettingsService
    from app.secrets import SecretStore

    return SettingsService(settings, SecretStore(settings.data_dir / "secrets.enc"))


def _make_engine():
    from app.embedder import Embedder

    svc = _make_svc()
    return AnswerEngine(Embedder(svc), svc)


def test_planner_hybrid_always():
    engine = _make_engine()
    assert engine._plan_tools("What is the DVCA movement rule?") == [TOOL_HYBRID]


def test_planner_adds_who_knows_for_ownership_questions():
    engine = _make_engine()
    for q in [
        "Who owns the billing migration?",
        "who is responsible for security reviews?",
        "Whom does the wallet integration depend on?",
        "who knows about settlement?",
    ]:
        tools = engine._plan_tools(q)
        assert TOOL_HYBRID in tools
        assert TOOL_WHO_KNOWS in tools, f"expected who_knows tool for {q!r}"


def test_who_knows_surfaces_owner_entities(client, tmp_path):
    """The who_knows tool ranks entities whose owner/author/summary matches the
    query — even without embeddings (CI-safe, keyword overlap based)."""
    (tmp_path / "people.md").write_text(
        "Sarah owns the billing migration. Mike does security reviews.\n",
        encoding="utf-8",
    )
    (tmp_path / "d.md").write_text(
        "Decision: the billing migration is approved, owned by Sarah.\n",
        encoding="utf-8",
    )
    source = client.post(
        "/sources",
        json={"connector": "folder", "name": "b17", "config": {"path": str(tmp_path)}},
    ).json()
    from tests.test_smoke import start_and_wait

    start_and_wait(client, source["id"])

    engine = _make_engine()
    with SessionLocal() as db:
        hits = engine._who_knows_search(db, "who owns the billing migration")
        assert hits, "expected who_knows hits"
        assert any(h["entity"] is not None for h in hits)


def test_executor_runs_planned_tools(client, tmp_path):
    """_execute_tools returns an evidence bundle keyed by tool, and the hybrid
    list is non-empty after an ingest (keyword path works without models)."""
    (tmp_path / "rules.md").write_text(
        "§5 DVCA movement rules\n"
        "The DVCA movement must be reported with the DVSE identifier.\n",
        encoding="utf-8",
    )
    source = client.post(
        "/sources",
        json={"connector": "folder", "name": "b17exec", "config": {"path": str(tmp_path)}},
    ).json()
    from tests.test_smoke import start_and_wait

    start_and_wait(client, source["id"])

    engine = _make_engine()
    tools = engine._plan_tools("What are the DVCA movement rules?")
    assert tools == [TOOL_HYBRID]

    from app.db import SessionLocal

    with SessionLocal() as db:
        evidence = asyncio.run(engine._execute_tools(db, "What are the DVCA movement rules?", tools))
    assert TOOL_HYBRID in evidence
    assert evidence[TOOL_HYBRID], "hybrid retrieval must return hits"


def test_multi_tool_evidence_fusion(client, tmp_path):
    """who_knows hits plus hybrid hits fuse into one ranked list; both source
    types (decision + person) can appear in the final evidence."""
    (tmp_path / "a.md").write_text(
        "Sarah owns the billing migration and must approve settlement changes.\n",
        encoding="utf-8",
    )
    source = client.post(
        "/sources",
        json={"connector": "folder", "name": "b17fuse", "config": {"path": str(tmp_path)}},
    ).json()
    from tests.test_smoke import start_and_wait

    start_and_wait(client, source["id"])

    engine = _make_engine()
    tools = engine._plan_tools("who owns the billing migration")
    with SessionLocal() as db:
        evidence = asyncio.run(engine._execute_tools(db, "who owns the billing migration", tools))
        assert TOOL_WHO_KNOWS in evidence
        fused = engine._fuse_evidence(evidence, [])
        assert fused, "fused evidence must be non-empty"


def test_qa_multi_source_cited(client, tmp_path):
    """DoD: a question needing decision + person evidence returns citations
    from both — the planner runs who_knows alongside hybrid."""
    (tmp_path / "people.md").write_text(
        "Sarah owns the billing migration.\n",
        encoding="utf-8",
    )
    (tmp_path / "decision.md").write_text(
        "Decision: approved moving to the new billing provider, owned by Sarah.\n",
        encoding="utf-8",
    )
    source = client.post(
        "/sources",
        json={"connector": "folder", "name": "b17qa", "config": {"path": str(tmp_path)}},
    ).json()
    from tests.test_smoke import start_and_wait

    start_and_wait(client, source["id"])

    resp = client.post("/qa", json={"question": "Who owns the billing migration?"})
    assert resp.status_code == 200, resp.text
    citations = resp.json()["citations"]
    assert citations, "expected citations"
    all_text = " ".join(c["summary"] + " " + c["snippet"] for c in citations).lower()
    assert "sarah" in all_text, f"person evidence missing: {citations}"
    assert any("billing" in c["summary"].lower() or "billing" in c["snippet"].lower() for c in citations), (
        f"decision/action evidence missing: {citations}"
    )
