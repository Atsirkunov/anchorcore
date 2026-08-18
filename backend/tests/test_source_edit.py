"""B7: source configuration editing.

Verifies the DoD: editing a source (name, enabled, config) works via PUT and
secret fields stay keychain-backed ('***set***' keeps the stored value).
"""

from tests.test_smoke import start_and_wait


def _mk_folder(client, tmp_path, name: str) -> dict:
    (tmp_path / "d.md").write_text("We decided to keep this source fresh.", encoding="utf-8")
    return client.post(
        "/sources",
        json={"connector": "folder", "name": name, "config": {"path": str(tmp_path)}},
    ).json()


def test_update_name_and_enabled(client, tmp_path):
    source = _mk_folder(client, tmp_path, "old-name")
    resp = client.put(f"/sources/{source['id']}", json={"name": "new-name", "enabled": False})
    assert resp.status_code == 200, resp.text
    updated = resp.json()
    assert updated["name"] == "new-name"
    assert updated["enabled"] is False

    # partial update leaves other fields alone
    resp = client.put(f"/sources/{source['id']}", json={"name": "renamed-again"})
    assert resp.json()["name"] == "renamed-again"
    assert resp.json()["enabled"] is False

    # empty name → 422, unknown source → 404
    assert client.put(f"/sources/{source['id']}", json={"name": "  "}).status_code == 422
    assert client.put("/sources/999999", json={"name": "x"}).status_code == 404


def test_update_folder_config(client, tmp_path):
    source = _mk_folder(client, tmp_path, "folder-edit")
    assert client.get(f"/sources/{source['id']}/config").json()["path"] == str(tmp_path)

    other = tmp_path.parent / "other"
    other.mkdir(exist_ok=True)
    (other / "b.md").write_text("We decided to point here now.", encoding="utf-8")

    resp = client.put(f"/sources/{source['id']}", json={"config": {"path": str(other)}})
    assert resp.status_code == 200, resp.text
    assert client.get(f"/sources/{source['id']}/config").json()["path"] == str(other)

    # a sync against the new path succeeds
    job = start_and_wait(client, source["id"])
    assert job["status"] == "done"
    assert job["result"]["items"] == 1


def test_secret_fields_stay_keychain_backed(client):
    source = client.post(
        "/sources",
        json={
            "connector": "jira",
            "name": "jira-secret",
            "config": {"base_url": "https://x.atlassian.net", "email": "a@b.c", "token": "orig-token"},
        },
    ).json()

    def stored_token() -> str:
        from app.config import settings
        from app.secrets import SecretStore

        return SecretStore(settings.data_dir / "secrets.enc").get(f"source:{source['id']}:token") or ""

    assert stored_token() == "orig-token"

    # the masked placeholder keeps the stored value untouched
    resp = client.put(f"/sources/{source['id']}", json={"config": {"base_url": "https://y.atlassian.net", "token": "***set***"}})
    assert resp.status_code == 200, resp.text
    assert stored_token() == "orig-token"
    cfg = client.get(f"/sources/{source['id']}/config").json()
    assert cfg["token"] == "***set***"
    assert cfg["base_url"] == "https://y.atlassian.net"

    # a real value replaces it
    client.put(f"/sources/{source['id']}", json={"config": {"token": "new-token"}})
    assert stored_token() == "new-token"

    # clearing the field removes the stored secret
    client.put(f"/sources/{source['id']}", json={"config": {"token": ""}})
    assert stored_token() == ""


def test_new_secret_field_auto_masked(client, tmp_path, monkeypatch):
    """B34 DoD: adding a field to SECRET_SOURCE_FIELDS must auto-mask it in
    GET /sources/{id}/config (single source of truth — no divergence)."""
    from app import secrets as secrets_module

    original = secrets_module.SECRET_SOURCE_FIELDS
    monkeypatch.setattr(secrets_module, "SECRET_SOURCE_FIELDS", original + ("client_secret",))

    source = client.post(
        "/sources",
        json={
            "connector": "jira",
            "name": "auto-mask",
            "config": {"base_url": "https://x.atlassian.net", "client_secret": "shh-secret"},
        },
    ).json()

    cfg = client.get(f"/sources/{source['id']}/config").json()
    assert cfg["client_secret"] == "***set***", "new secret field must be masked"
    assert "shh-secret" not in str(cfg), "raw secret must never reach the UI"

