#!/usr/bin/env bash
set -euo pipefail

# R2.3 conformance harness: start Rust on :8123 and run backend/tests against it via HTTP
# Usage: ./rust/scripts/conformance.sh
# Requires: cargo, pytest, ANCHOR_DATA_DIR shared via conftest temp DB

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
RUST_DIR="$ROOT/rust"
# backend/ (tests + venv) lives on the archive tag — restore it if missing
if [ ! -d "$ROOT/backend" ]; then
  git -C "$ROOT" fetch origin tag archive/python-final --no-tags
  git -C "$ROOT" checkout archive/python-final -- backend
fi
PORT="${ANCHOR_TEST_PORT:-8123}"
URL="http://127.0.0.1:$PORT"

echo "=> Starting Rust anchorcore on $URL (data dir from pytest conftest)..."
# Start Rust in background; it will inherit ANCHOR_DATA_DIR / ANCHOR_DATABASE_URL from pytest's env
# We need to run pytest first to get its temp DB path, then start Rust with same env - so we do it inside pytest via fixture.
# Simpler: start Rust with a temp data dir and run the conformance tests that create their own DB entries.

# For now, just start Rust with default data/ and run the two conformance tests
source "$HOME/.cargo/env" 2>/dev/null || true

# Build first
cargo build --manifest-path "$RUST_DIR/Cargo.toml" -p anchorcore

# Start Rust
ANCHOR_DATA_DIR="${ANCHOR_DATA_DIR:-$ROOT/data}" cargo run --manifest-path "$RUST_DIR/Cargo.toml" -p anchorcore -- --port "$PORT" --data-dir "${ANCHOR_DATA_DIR:-$ROOT/data}" &
RUST_PID=$!
trap "kill $RUST_PID 2>/dev/null || true" EXIT

echo "=> Waiting for $URL/health..."
for i in {1..20}; do
  if curl -s "$URL/health" | grep -q '"status":"ok"'; then
    echo "=> Rust ready"
    break
  fi
  sleep 0.5
done

echo "=> Running Python conformance tests against Rust..."
PYTEST_BIN="$ROOT/backend/.venv/bin/pytest"
command -v pytest >/dev/null 2>&1 && PYTEST_BIN="pytest"
ANCHOR_TEST_RUST_URL="$URL" "$PYTEST_BIN" "$ROOT/backend/tests/test_rust_conformance.py" -v
# Also run pure Rust retrieval unit tests
cargo test --manifest-path "$RUST_DIR/Cargo.toml" -p anchorcore -- --nocapture

echo "=> Conformance done"
kill $RUST_PID
wait $RUST_PID 2>/dev/null || true
