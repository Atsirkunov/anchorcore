#!/usr/bin/env python3
"""Sync single source version: rust/Cargo.toml -> backend/app/config.py

Cutover single source is rust/Cargo.toml workspace.package.version (1.0.8).
This script ensures backend/app/config.py __version__ matches, or vice versa with --reverse.
Use in CI: python scripts/sync_version.py --check
"""
import argparse
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
if not (ROOT / "rust" / "Cargo.toml").exists():
    ROOT = Path(__file__).resolve().parents[2]
CARGO = ROOT / "rust" / "Cargo.toml"
CONFIG = ROOT / "backend" / "app" / "config.py"

def read_cargo_version() -> str:
    text = CARGO.read_text()
    # workspace.package.version = "1.0.8"
    m = re.search(r'^\s*version\s*=\s*"([^"]+)"', text, re.MULTILINE)
    if not m:
        print(f"no version in {CARGO}", file=sys.stderr)
        sys.exit(1)
    return m.group(1)

def read_config_version() -> str:
    text = CONFIG.read_text()
    m = re.search(r'__version__\s*=\s*"([^"]+)"', text)
    if not m:
        print(f"no __version__ in {CONFIG}", file=sys.stderr)
        sys.exit(1)
    return m.group(1)

def write_config_version(new: str):
    text = CONFIG.read_text()
    new_text, n = re.subn(r'(__version__\s*=\s*)"[^"]+"', rf'\1"{new}"', text, count=1)
    if n == 0:
        print("failed to update config", file=sys.stderr)
        sys.exit(1)
    CONFIG.write_text(new_text)
    print(f"updated {CONFIG} -> {new}")

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--check", action="store_true", help="exit 1 if mismatch")
    ap.add_argument("--reverse", action="store_true", help="sync Cargo from config (not default)")
    args = ap.parse_args()
    cargo_v = read_cargo_version()
    config_v = read_config_version()
    if args.reverse:
        # update Cargo from config
        if cargo_v != config_v:
            if args.check:
                print(f"mismatch: Cargo {cargo_v} != config {config_v}", file=sys.stderr)
                sys.exit(1)
            # update Cargo
            text = CARGO.read_text()
            new_text = re.sub(r'(^\s*version\s*=\s*)"[^"]+"', rf'\1"{config_v}"', text, count=1, flags=re.MULTILINE)
            CARGO.write_text(new_text)
            print(f"updated {CARGO} -> {config_v}")
        else:
            print(f"ok {cargo_v}")
    else:
        if cargo_v != config_v:
            if args.check:
                print(f"mismatch: Cargo {cargo_v} != config {config_v}", file=sys.stderr)
                sys.exit(1)
            write_config_version(cargo_v)
        else:
            print(f"ok {cargo_v}")

if __name__ == "__main__":
    main()
