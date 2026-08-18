"""B30 (partial): PII knowledge base, config, and chunk review."""

import json

from app.pii import categories_payload, load_config, save_config, scan_text, serialize_matches


def test_scan_recognises_common_pii():
    text = (
        "Contact joe.bloggs@example.com or 555-123-4567. "
        "SSN 123-45-6789, card 4111-1111-1111-1111, password=hunter2."
    )
    matches = scan_text(text)
    by_cat = {m.category: m for m in matches}
    assert by_cat["email"].strong and by_cat["email"].match == "joe.bloggs@example.com"
    assert by_cat["phone"].strong
    assert by_cat["ssn"].strong
    assert by_cat["credit_card"].strong
    assert by_cat["credentials"].strong


def test_scan_field_name_without_value_is_weak():
    """A named field ('email:') without a validating pattern is a weak signal."""
    matches = scan_text("Please send it to the email on file.")
    by_cat = {m.category: m for m in matches}
    assert "email" in by_cat
    assert by_cat["email"].strong is False


def test_scan_custom_words_and_disabled_categories():
    text = "the project lead sam rivers approved this budget email x@y.z"
    m1 = scan_text(text, custom_words={"sam rivers"})
    assert any(m.category == "custom" and m.label == "sam rivers" for m in m1)

    m2 = scan_text("reach out by email to a@b.co", disabled={"email"})
    assert "email" not in {m.category for m in m2}

    m3 = scan_text("email a@b.co", custom_words=set())
    assert "email" in {m.category for m in m3}


def test_serialize_matches_shape():
    matches = scan_text("joe@x.com")
    payload = serialize_matches(matches)
    assert payload[0]["category"] == "email"
    assert payload[0]["strong"] is True
    assert "match" in payload[0]


def test_config_roundtrip(client):
    resp = client.get("/pii/config")
    assert resp.status_code == 200, resp.text
    body = resp.json()
    assert "categories" in body and "custom_words" in body
    assert any(c["id"] == "email" and c["enabled"] for c in body["categories"])
    assert body["custom_words"] == []

    put = client.put("/pii/config", json={"custom_words": ["sam rivers"], "disabled_categories": ["email"]})
    assert put.status_code == 200, put.text
    assert put.json()["custom_words"] == ["sam rivers"]
    email_cat = next(c for c in put.json()["categories"] if c["id"] == "email")
    assert email_cat["enabled"] is False

    # persisted
    again = client.get("/pii/config").json()
    assert again["custom_words"] == ["sam rivers"]
    assert next(c for c in again["categories"] if c["id"] == "email")["enabled"] is False

    # cleanup
    client.put("/pii/config", json={"custom_words": [], "disabled_categories": []})


def test_scan_endpoint_flags_and_review(client, tmp_path):
    from tests.test_smoke import start_and_wait

    (tmp_path / "p.md").write_text(
        "Decision: the finance contact is joe.bloggs@example.com, card 4111-1111-1111-1111.",
        encoding="utf-8",
    )
    source = client.post(
        "/sources",
        json={"connector": "folder", "name": "pii-doc", "config": {"path": str(tmp_path)}},
    ).json()
    job = start_and_wait(client, source["id"])
    assert job["status"] == "done", job

    review = client.get("/pii/review").json()
    flagged = [r for r in review if "joe.bloggs@example.com" in r["content"]]
    assert flagged, "expected a chunk flagged for the email/card PII"
    assert flagged[0]["is_pii"] is True
    cats = {c["category"] for c in flagged[0]["categories"]}
    assert {"email", "credit_card"} <= cats

    # confirm the classification via the review endpoint
    chunk_id = flagged[0]["chunk_id"]
    decide = client.post(f"/pii/review/{chunk_id}", json={"is_pii": True})
    assert decide.status_code == 200, decide.text
    assert decide.json()["is_pii"] is True

    # source scan endpoint runs without error
    scan = client.post(f"/pii/scan/{source['id']}")
    assert scan.status_code == 200, scan.text
    assert scan.json()["chunks"] >= 1
    assert scan.json()["flagged"] >= 1
