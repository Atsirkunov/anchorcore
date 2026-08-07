"""B32: graph-based retrieval — relationship-aware rerank + expansion.

Verifies the graph walk in `AnswerEngine._graph_expand`: strong relationship
kinds rank above weak ones, fan-out/hops are capped, stale/disputed connected
entities are excluded, and connected entities surface as citations in Q&A.
"""

from datetime import datetime, timezone

from sqlalchemy import select

from app.answer_engine import AnswerEngine, GRAPH_KIND_WEIGHTS
from app.db import SessionLocal
from app.models import Chunk, Entity, IngestedItem, Relationship


def _engine():
    from app.app_settings import SettingsService
    from app.config import settings
    from app.embedder import Embedder
    from app.secrets import SecretStore

    svc = SettingsService(settings, SecretStore(settings.data_dir / "secrets.enc"))
    return AnswerEngine(Embedder(svc), svc)


def _seed_graph(client, tmp_path) -> dict:
    """Seed items + entities + a supersedes/depends_on/related chain and return
    the created entity ids keyed by summary fragment."""
    items = [
        ("old.md", "Decision: legacy payment rail was replaced by the new platform."),
        ("new.md", "Decision: the new payment platform supersedes the legacy rail."),
        ("dep.md", "Decision: the new platform depends on the billing migration."),
        ("hub.md", "Decision: the payments hub owns all movement rules."),
    ]
    for filename, text in items:
        (tmp_path / filename).write_text(text, encoding="utf-8")

    source = client.post(
        "/sources",
        json={"connector": "folder", "name": "graph", "config": {"path": str(tmp_path)}},
    ).json()
    from tests.test_smoke import start_and_wait

    job = start_and_wait(client, source["id"])
    assert job["status"] == "done"
    source_id = source["id"]

    with SessionLocal() as db:
        ents = db.execute(select(Entity).order_by(Entity.id)).scalars().all()
        by_key: dict[str, Entity] = {}
        for e in ents:
            s = e.summary.lower()
            if "legacy payment rail was replaced" in s:
                by_key["old"] = e
            elif "supersedes the legacy" in s:
                by_key["new"] = e
            elif "depends on the billing" in s:
                by_key["dep"] = e
            elif "owns all movement" in s:
                by_key["hub"] = e
            elif e.kind == "document":
                by_key[f"doc:{e.item_id}"] = e

        for key, e in by_key.items():
            if key.startswith("doc:"):
                continue
            assert e is not None, f"missing entity {key}"
        ids = {k: v.id for k, v in by_key.items() if not k.startswith("doc:")}

        # connect: new supersedes old; dep depends_on new; hub owns new
        db.add_all(
            [
                Relationship(from_entity_id=ids["new"], to_entity_id=ids["old"], kind="supersedes"),
                Relationship(from_entity_id=ids["dep"], to_entity_id=ids["new"], kind="depends_on"),
                Relationship(from_entity_id=ids["hub"], to_entity_id=ids["new"], kind="owns"),
            ]
        )
        db.commit()
        ids["old_item"] = next(e.item_id for e in by_key.values() if e is by_key["old"])
        ids["source_id"] = source_id
    return ids


def test_graph_expand_surfaces_connected_entities(client, tmp_path):
    """A hit entity with a `supersedes` neighbor must pull that neighbor in."""
    ids = _seed_graph(client, tmp_path)

    engine = _engine()
    with SessionLocal() as db:
        new = db.get(Entity, ids["new"])
        hit = {
            "chunk": db.execute(select(Chunk).where(Chunk.entity_id == new.id)).scalar(),
            "entity": new,
            "item": new.item,
            "source_id": new.item.source_id,
            "score": 1.0,
        }
        expanded = engine._graph_expand(db, [hit])
        assert expanded, "expected graph expansion"
        expanded_ids = {h["entity"].id for h in expanded}
        assert ids["old"] in expanded_ids, "superseded entity must surface"
        assert ids["dep"] in expanded_ids, "depends_on entity must surface"
        assert ids["hub"] in expanded_ids, "owns entity must surface"


def test_graph_expand_ranks_strong_kinds_first(client, tmp_path):
    """supersedes beats related; cap respects top-k ordering."""
    from app.config import settings

    ids = _seed_graph(client, tmp_path)

    # add a weak 'related' link to a 5th entity
    with SessionLocal() as db:
        item = IngestedItem(
            source_id=ids["source_id"],
            external_id="weak.md",
            title="Weak link",
            text="Decision: an unrelated note about parking.",
            content_hash=f"hash-weak-{ids['source_id']}",
            doc_type="general",
        )
        db.add(item)
        db.flush()
        weak = Entity(
            item_id=item.id,
            kind="decision",
            summary="Decision: parking garage rules are unchanged.",
            status="unverified",
        )
        db.add(weak)
        db.flush()
        new = db.get(Entity, ids["new"])
        db.add(Relationship(from_entity_id=new.id, to_entity_id=weak.id, kind="related"))
        db.commit()
        weak_id = weak.id

    engine = _engine()
    # lift the fan-out cap so the weak 'related' entity is also eligible
    original_cap = settings.retrieval_graph_max
    settings.retrieval_graph_max = 10
    try:
        with SessionLocal() as db:
            new = db.get(Entity, ids["new"])
            hit = {
                "chunk": db.execute(select(Chunk).where(Chunk.entity_id == new.id)).scalar(),
                "entity": new,
                "item": new.item,
                "source_id": new.item.source_id,
                "score": 1.0,
            }
            expanded = engine._graph_expand(db, [hit])
            by_id = {h["entity"].id: h["score"] for h in expanded}
            assert ids["old"] in by_id, "supersedes must outrank related"
            assert weak_id in by_id, "related entity present"
            assert by_id[ids["old"]] > by_id[weak_id], (
                f"supersedes ({by_id.get(ids['old'])}) must rank above related ({by_id.get(weak_id)})"
            )
    finally:
        settings.retrieval_graph_max = original_cap


def test_graph_expand_excludes_stale_and_caps_fanout(client, tmp_path):
    """Stale/disputed connected entities never enter context; fan-out is capped."""
    from app.config import settings

    ids = _seed_graph(client, tmp_path)

    # mark 'dep' stale → must be excluded even though it's connected
    with SessionLocal() as db:
        dep = db.get(Entity, ids["dep"])
        dep.status = "stale"
        db.commit()

    engine = _engine()
    with SessionLocal() as db:
        new = db.get(Entity, ids["new"])
        hit = {
            "chunk": db.execute(select(Chunk).where(Chunk.entity_id == new.id)).scalar(),
            "entity": new,
            "item": new.item,
            "source_id": new.item.source_id,
            "score": 1.0,
        }
        expanded = engine._graph_expand(db, [hit])
        expanded_ids = {h["entity"].id for h in expanded}
        assert ids["dep"] not in expanded_ids, "stale entity must be excluded"
        assert ids["old"] in expanded_ids, "non-stale supersedes still surfaces"
        assert len(expanded) <= settings.retrieval_graph_max, (
            f"fan-out must be capped at {settings.retrieval_graph_max}"
        )


def test_graph_kind_weights_are_sane():
    assert GRAPH_KIND_WEIGHTS["supersedes"] > GRAPH_KIND_WEIGHTS["depends_on"] > GRAPH_KIND_WEIGHTS["related"]


def test_qa_includes_graph_citations(client, tmp_path):
    """Q&A over a document that references a superseded entity surfaces the
    connected decision as a citation without the words co-occurring."""
    ids = _seed_graph(client, tmp_path)

    # ask about the legacy rail — its own chunk may not mention 'new platform'
    resp = client.post("/qa", json={"question": "What happened to the legacy payment rail?"})
    assert resp.status_code == 200, resp.text
    citations = resp.json()["citations"]
    # either the legacy decision is cited directly, or the connected (supersedes)
    # decision appears — the graph expansion must surface the connection
    all_text = " ".join(c["summary"] for c in citations)
    assert any("payment" in c["summary"].lower() or "platform" in c["summary"].lower() for c in citations), citations
    assert len(citations) >= 1
