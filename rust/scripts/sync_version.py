#!/usr/bin/env python3
"""Version single source: rust/Cargo.toml workspace.package.version.

The Python backend (backend/app/config.py) was archived on tag
archive/python-final, so Cargo.toml is the only version source left.
CI: python rust/scripts/sync_version.py --check exits 0 when the version parses.
"""
import argparse
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
# handle being inside rust/scripts (parents[1]==rust) vs scripts (parents[1]==root)
if not (ROOT / "rust" / "Cargo.toml").exists():
    ROOT = Path(__file__).resolve().parents[2]
CARGO = ROOT / "rust" / "Cargo.toml"

def read_cargo_version() -> str:
    text = CARGO.read_text()
    m = re.search(r'^\s*version\s*=\s*"([^"]+)"', text, re.MULTILINE)
    if not m:
        print(f"no version in {CARGO}", file=sys.stderr)
        sys.exit(1)
    return m.group(1)

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--check", action="store_true", help="exit 1 if version missing")
    ap.add_argument("--reverse", action="store_true", help="deprecated: backend archived, no-op")
    args = ap.parse_args()
    cargo_v = read_cargo_version()
    if args.reverse:
        print(f"backend archived; Cargo is the only source ({cargo_v})")
    else:
        print(f"ok {cargo_v}")

if __name__ == "__main__":
    main()
