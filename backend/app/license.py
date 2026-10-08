"""Team edition license check — offline, no phone-home.

The personal local app never touches this module. The Docker (team)
edition calls :func:`enforce_team_license` at startup: a missing or
forged license refuses to boot, an expired maintenance window boots
with a warning (perpetual license — support lapses, the app keeps
running).

License file format (JSON)::

    {"payload": {"org": ..., "edition": "team", "issued": ...,
                 "maintenance_until": ..., "key_id": ...},
     "signature": "<base64 ed25519 over canonical JSON of payload>"}

Canonical form is ``json.dumps(payload, sort_keys=True,
separators=(",", ":"))``. Use ``scripts/make_license.py`` to issue.
"""
from __future__ import annotations

import base64
import binascii
import datetime as _dt
import json
import logging
import os
from dataclasses import dataclass
from pathlib import Path
from typing import Literal

from cryptography.exceptions import InvalidSignature
from cryptography.hazmat.primitives.asymmetric import ed25519

logger = logging.getLogger(__name__)

# Ed25519 public key (raw 32 bytes, hex). The private half lives with the
# seller only — it must never be committed. Rotate by appending here and
# matching `key_id` prefixes in make_license.py.
PUBLIC_KEYS_HEX = (
    "37c9b28f28948a06b7d3a9b9b32b799fd6f4a018ccaaffb12ad89950e1b0ca3e",
)

TEAM_EDITION = "team"
STATUS = Literal["ok", "lapsed", "missing", "invalid"]


@dataclass(frozen=True)
class LicenseResult:
    status: STATUS
    org: str | None
    maintenance_until: _dt.date | None
    message: str


def _canonical(payload: dict) -> bytes:
    return json.dumps(payload, sort_keys=True, separators=(",", ":")).encode("utf-8")


def verify_license(
    raw: str | None,
    *,
    public_keys_hex: tuple[str, ...] | None = None,
    today: _dt.date | None = None,
) -> LicenseResult:
    """Verify a license document. Pure function — no env, no I/O."""
    day = today or _dt.date.today()
    public_keys_hex = public_keys_hex if public_keys_hex is not None else PUBLIC_KEYS_HEX
    if not raw or not raw.strip():
        return LicenseResult("missing", None, None, "no license provided")
    try:
        doc = json.loads(raw)
        payload = doc["payload"]
        sig = base64.b64decode(doc["signature"])
        assert isinstance(payload, dict) and payload.get("edition") == TEAM_EDITION
        until = _dt.date.fromisoformat(payload["maintenance_until"])
        org = str(payload.get("org") or "unknown")
    except (json.JSONDecodeError, KeyError, AssertionError, ValueError, binascii.Error) as e:
        return LicenseResult("invalid", None, None, f"malformed license: {e}")
    keys = []
    for h in public_keys_hex:
        try:
            keys.append(ed25519.Ed25519PublicKey.from_public_bytes(bytes.fromhex(h)))
        except ValueError:
            continue
    if not keys:
        return LicenseResult("invalid", None, None, "no valid public key configured")
    for key in keys:
        try:
            key.verify(sig, _canonical(payload))
            break
        except InvalidSignature:
            continue
    else:
        return LicenseResult("invalid", org, until, "signature mismatch")
    if until < day:
        return LicenseResult(
            "lapsed", org, until,
            f"maintenance expired {until.isoformat()} — app runs, support/updates lapsed",
        )
    return LicenseResult("ok", org, until, f"licensed to {org} until {until.isoformat()}")


def load_license_from_env() -> LicenseResult:
    """Read from ANCHOR_LICENSE_FILE (path) or ANCHOR_LICENSE (inline JSON)."""
    path = os.environ.get("ANCHOR_LICENSE_FILE")
    if path:
        try:
            return verify_license(Path(path).read_text(encoding="utf-8"))
        except OSError as e:
            return LicenseResult("missing", None, None, f"cannot read {path}: {e}")
    return verify_license(os.environ.get("ANCHOR_LICENSE"))


def enforce_team_license() -> LicenseResult:
    """Startup gate for the Docker edition. Raises RuntimeError on refusal."""
    result = load_license_from_env()
    if result.status == "ok":
        logger.info("Team license: %s", result.message)
    elif result.status == "lapsed":
        logger.warning("Team license lapsed: %s", result.message)
    else:
        raise RuntimeError(
            f"Team edition requires a license ({result.message}). "
            "Mount it at ANCHOR_LICENSE_FILE or set ANCHOR_LICENSE. "
            "Contact sales at alex@anchorcore.dev for a platform license."
        )
    return result
