"""R2.3: Rust conformance harness — verifies Rust binary matches Python contract via HTTP.

Run:
  ANCHOR_TEST_RUST_URL=http://127.0.0.1:8123 pytest backend/tests/test_rust_conformance.py -v
Or via script:
  rust/scripts/conformance.sh

Requires Rust binary running (cargo run -p anchorcore -- --port 8123).
"""

import os
import httpx
import pytest


def _rust_client():
    url = os.environ.get("ANCHOR_TEST_RUST_URL", "http://127.0.0.1:8123")
    return httpx.Client(base_url=url, timeout=10.0)


def test_rust_health_matches_python_shape():
    rus = os.environ.get("ANCHOR_TEST_RUST_URL")
    if not rus:
        pytest.skip("ANCHOR_TEST_RUST_URL not set — run via rust/scripts/conformance.sh")
    c = _rust_client()
    r = c.get("/health")
    assert r.status_code == 200, r.text
    j = r.json()
    assert j["status"] == "ok"
    assert "data_dir" in j
    assert "components" in j
    assert "failing_sources" in j["components"]
    assert "pending_embeddings" in j["components"]
    c.close()


def test_rust_qa_keyword_conformance(tmp_path):
    rus = os.environ.get("ANCHOR_TEST_RUST_URL")
    if not rus:
        pytest.skip("ANCHOR_TEST_RUST_URL not set")
    import sqlite3
    from pathlib import Path

    # Get Rust's data_dir via /health so we share the same DB file
    c0 = _rust_client()
    try:
        h = c0.get("/health").json()
        data_dir = Path(h.get("data_dir", "data"))
        db_path = data_dir / "anchorcore.db"
    finally:
        c0.close()
    if not db_path.exists():
        pytest.skip(f"DB not found at {db_path}")

    # Insert a test chunk directly (bypass pipeline, like test_vec0)
    conn = sqlite3.connect(str(db_path))
    conn.execute("INSERT OR IGNORE INTO sources (id, connector, name, config) VALUES (999, 'folder', 'rust-conf', '{}')")
    # ensure FTS and vec triggers exist
    conn.execute("INSERT INTO chunks (id, content, source_ref, is_pii) VALUES (9999, 'Rust conformance DVCA test content for keyword search', 'test', 0) ON CONFLICT(id) DO UPDATE SET content=excluded.content")
    conn.commit()
    conn.close()

    c = _rust_client()
    r = c.post("/qa", json={"question": "DVCA"})
    assert r.status_code == 200, r.text
    j = r.json()
    assert "answer" in j and "citations" in j
    # Rust should find the DVCA chunk via FTS
    assert any("DVCA" in cit.get("snippet", "") or "DVCA" in cit.get("summary", "") for cit in j["citations"]) or "DVCA" in j["answer"], j

    # cleanup
    conn = sqlite3.connect(str(db_path))
    conn.execute("DELETE FROM chunks WHERE id = 9999")
    conn.execute("DELETE FROM sources WHERE id = 999")
    conn.commit()
    conn.close()
    c.close()
