"""B39: security/distribution — sources.label + provider-trust gate.

The thin gate (before full B30): a source labelled `sensitive`/`pii` must not
be sent to an unconfirmed cloud classifier/embedder. Locally-configured
providers are always trusted, so the suite (Ollama at localhost:1) sees no
behaviour change by default — these tests exercise the gate decision logic
directly and the label plumbing end-to-end.
"""

from app.status import cloud_classifier_trusted, cloud_embedder_trusted, sensitive_label


def test_sensitive_labels_flagged():
    assert sensitive_label("pii") is True
    assert sensitive_label("sensitive") is True
    assert sensitive_label("internal") is False
    assert sensitive_label("public") is False


def test_local_providers_always_trusted(client):
    """Local (localhost) providers are trusted regardless of ANCHOR_CLOUD_TRUST."""
    import os

    from app.app_settings import SettingsService
    from app.config import settings
    from app.secrets import SecretStore

    svc = SettingsService(settings, SecretStore(settings.data_dir / "secrets.enc"))
    assert cloud_classifier_trusted(svc) is True  # classifier_base_url → ollama (localhost)
    assert cloud_embedder_trusted(svc) is True

    # even with the trust flag explicitly OFF, local is trusted
    os.environ["ANCHOR_CLOUD_TRUST"] = "0"
    try:
        assert cloud_classifier_trusted(svc) is True
    finally:
        os.environ.pop("ANCHOR_CLOUD_TRUST", None)


def test_remote_provider_untrusted_without_flag(client):
    """A cloud provider is only trusted once ANCHOR_CLOUD_TRUST=1."""
    from app.app_settings import SettingsService
    from app.config import settings
    from app.secrets import SecretStore

    svc = SettingsService(settings, SecretStore(settings.data_dir / "secrets.enc"))
    # simulate a cloud classifier URL
    svc.set("classifier_base_url", "https://api.openai.com/v1")
    svc.set("embed_base_url", "https://api.openai.com/v1")
    try:
        assert cloud_classifier_trusted(svc) is False
        assert cloud_embedder_trusted(svc) is False
        import os

        os.environ["ANCHOR_CLOUD_TRUST"] = "1"
        try:
            assert cloud_classifier_trusted(svc) is True
            assert cloud_embedder_trusted(svc) is True
        finally:
            os.environ.pop("ANCHOR_CLOUD_TRUST", None)
    finally:
        svc.clear("classifier_base_url")
        svc.clear("embed_base_url")


def test_source_label_plumbed_through_api(client, tmp_path):
    """B39: create + update a source with a label; invalid labels rejected."""
    (tmp_path / "a.md").write_text("internal note", encoding="utf-8")
    source = client.post(
        "/sources",
        json={
            "connector": "folder",
            "name": "pii-source",
            "label": "pii",
            "config": {"path": str(tmp_path)},
        },
    ).json()
    assert source["label"] == "pii"

    updated = client.put(f"/sources/{source['id']}", json={"label": "public"}).json()
    assert updated["label"] == "public"

    bad = client.post(
        "/sources",
        json={"connector": "folder", "name": "bad-label", "label": "secret-ish", "config": {}},
    )
    assert bad.status_code == 422, bad.text

    listed = client.get("/sources").json()
    assert any(s["id"] == source["id"] and s["label"] == "public" for s in listed)


def test_sensitive_source_still_ingests_with_rules(client, tmp_path):
    """B39 DoD (local providers): a pii-labelled source ingests fine — local
    providers are trusted, so nothing is refused."""
    from tests.test_smoke import start_and_wait

    (tmp_path / "p.md").write_text(
        "We decided to redact personal details in the customer report.",
        encoding="utf-8",
    )
    source = client.post(
        "/sources",
        json={
            "connector": "folder",
            "name": "pii-ingest",
            "label": "pii",
            "config": {"path": str(tmp_path)},
        },
    ).json()
    job = start_and_wait(client, source["id"])
    assert job["status"] == "done", job
    entities = client.get("/entities").json()
    assert any(e["source_ref"].startswith(str(tmp_path)) for e in entities)
