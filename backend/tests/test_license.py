"""License verification: sign/verify round-trip, tamper, expiry, malformed."""
import base64
import datetime
import json

import pytest
from cryptography.hazmat.primitives.asymmetric import ed25519

from app.license import enforce_team_license, load_license_from_env, verify_license


def _keys():
    priv = ed25519.Ed25519PrivateKey.generate()
    return priv, (priv.public_key().public_bytes_raw().hex(),)


def _sign(priv, payload):
    raw = json.dumps(payload, sort_keys=True, separators=(",", ":")).encode()
    sig = base64.b64encode(priv.sign(raw)).decode()
    return json.dumps({"payload": payload, "signature": sig})


def _payload(**kw):
    today = datetime.date.today()
    base = {
        "org": "Acme",
        "edition": "team",
        "issued": today.isoformat(),
        "maintenance_until": (today + datetime.timedelta(days=365)).isoformat(),
        "key_id": "test",
    }
    base.update(kw)
    return base


def test_round_trip_ok():
    priv, (pub,) = _keys()
    r = verify_license(_sign(priv, _payload()), public_keys_hex=(pub,))
    assert r.status == "ok" and r.org == "Acme"


def test_tampered_payload_rejected():
    priv, (pub,) = _keys()
    doc = json.loads(_sign(priv, _payload()))
    doc["payload"]["org"] = "Mallory"  # modify after signing
    r = verify_license(json.dumps(doc), public_keys_hex=(pub,))
    assert r.status == "invalid"


def test_wrong_key_rejected():
    priv, _ = _keys()
    _, (other_pub,) = _keys()
    r = verify_license(_sign(priv, _payload()), public_keys_hex=(other_pub,))
    assert r.status == "invalid"


def test_lapsed_runs_with_warning():
    priv, (pub,) = _keys()
    yesterday = (datetime.date.today() - datetime.timedelta(days=1)).isoformat()
    r = verify_license(
        _sign(priv, _payload(maintenance_until=yesterday)), public_keys_hex=(pub,)
    )
    assert r.status == "lapsed" and r.maintenance_until is not None


def test_missing_and_malformed():
    assert verify_license(None).status == "missing"
    assert verify_license("").status == "missing"
    assert verify_license("{not json").status == "invalid"
    assert verify_license(json.dumps({"payload": {}, "signature": "AA=="})).status == "invalid"


def test_env_loading_and_enforcement(monkeypatch):
    priv, (pub,) = _keys()
    import app.license as lic

    monkeypatch.setattr(lic, "PUBLIC_KEYS_HEX", (pub,))
    monkeypatch.setenv("ANCHOR_LICENSE", _sign(priv, _payload()))
    monkeypatch.delenv("ANCHOR_LICENSE_FILE", raising=False)
    assert load_license_from_env().status == "ok"
    assert enforce_team_license().status == "ok"

    monkeypatch.setenv("ANCHOR_LICENSE", _sign(priv, _payload(org="X"))[:-4] + "AAAA")
    with pytest.raises(RuntimeError):
        enforce_team_license()

    monkeypatch.delenv("ANCHOR_LICENSE")
    with pytest.raises(RuntimeError):
        enforce_team_license()
