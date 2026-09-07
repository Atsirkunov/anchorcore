#!/usr/bin/env python3
"""Issue AnchorCore team licenses. Run on the seller machine only.

The private key must never be committed — pass it via
ANCHOR_LICENSE_PRIVATE_KEY (hex) or --private-key.

    # one-time setup: create the signing keypair
    python scripts/make_license.py gen-keypair

    # per sale: issue a license (maintenance = 1 year by default)
    export ANCHOR_LICENSE_PRIVATE_KEY=<hex from gen-keypair>
    python scripts/make_license.py issue --org "Acme Corp" --years 1 > license-acme.json

Ship license-acme.json to the customer; they mount it as
ANCHOR_LICENSE_FILE (or paste it into ANCHOR_LICENSE).
"""
from __future__ import annotations

import argparse
import base64
import datetime
import json
import os
import sys

from cryptography.hazmat.primitives.asymmetric import ed25519


def _canonical(payload: dict) -> bytes:
    return json.dumps(payload, sort_keys=True, separators=(",", ":")).encode("utf-8")


def gen_keypair() -> int:
    priv = ed25519.Ed25519PrivateKey.generate()
    pub = priv.public_key()
    print("PRIVATE (store in password manager, never commit):")
    print(priv.private_bytes_raw().hex())
    print("PUBLIC (embed in backend/app/license.py PUBLIC_KEYS_HEX):")
    print(pub.public_bytes_raw().hex())
    return 0


def issue(args: argparse.Namespace) -> int:
    hexkey = args.private_key or os.environ.get("ANCHOR_LICENSE_PRIVATE_KEY", "")
    try:
        priv = ed25519.Ed25519PrivateKey.from_private_bytes(bytes.fromhex(hexkey))
    except ValueError:
        print("error: invalid --private-key / ANCHOR_LICENSE_PRIVATE_KEY", file=sys.stderr)
        return 2
    today = datetime.date.today()
    payload = {
        "org": args.org,
        "edition": "team",
        "issued": today.isoformat(),
        "maintenance_until": (
            today + datetime.timedelta(days=int(args.years * 365))
        ).isoformat(),
        "key_id": priv.public_key().public_bytes_raw().hex()[:8],
    }
    sig = priv.sign(_canonical(payload))
    print(json.dumps({"payload": payload, "signature": base64.b64encode(sig).decode()}))
    return 0


def main() -> int:
    p = argparse.ArgumentParser(description=__doc__)
    sub = p.add_subparsers(dest="cmd", required=True)
    sub.add_parser("gen-keypair", help="create a new signing keypair")
    iss = sub.add_parser("issue", help="sign a team license")
    iss.add_argument("--org", required=True, help="customer org name")
    iss.add_argument("--years", type=float, default=1.0, help="maintenance window (default 1)")
    iss.add_argument("--private-key", default="", help="signing key hex (or env)")
    args = p.parse_args()
    return gen_keypair() if args.cmd == "gen-keypair" else issue(args)


if __name__ == "__main__":
    raise SystemExit(main())
