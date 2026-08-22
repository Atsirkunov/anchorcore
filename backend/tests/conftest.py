import os
import tempfile
from pathlib import Path

import pytest
from fastapi.testclient import TestClient

if "ANCHOR_DATABASE_URL" not in os.environ:
    _db_path = Path(tempfile.mkdtemp(prefix="anchorcore-")) / "test.db"
    os.environ["ANCHOR_DATABASE_URL"] = f"sqlite:///{_db_path}"
else:
    # Respect externally set DB (e.g. Rust conformance with shared file)
    _db_path = Path(os.environ["ANCHOR_DATABASE_URL"].removeprefix("sqlite:///"))

os.environ["ANCHOR_OLLAMA_BASE_URL"] = "http://localhost:1"
os.environ["ANCHOR_CLASSIFIER_TIMEOUT"] = "1.0"
os.environ["ANCHOR_HTTP_RETRIES"] = "0"
# Windows never RSTs closed localhost ports: each dead-port connect waits the
# full connect timeout twice (IPv6 + IPv4). Without this the suite crawls
# (~4s/lifespan probe, ~2s per model call). Real default stays 2.0s.
os.environ["ANCHOR_HTTP_CONNECT_TIMEOUT"] = "0.2"
os.environ["ANCHOR_ANSWER_BASE_URL"] = "https://api.openai.com/v1"
if "ANCHOR_DATA_DIR" not in os.environ:
    os.environ["ANCHOR_DATA_DIR"] = str(Path(tempfile.mkdtemp(prefix="anchorcore-data-")))
# never touch the real OS keychain from tests — secrets go to the encrypted
# fallback file under the temp data dir instead
os.environ["ANCHOR_SECRETS_NO_KEYRING"] = "1"


@pytest.fixture()
def client():
    # R2.3: if ANCHOR_TEST_RUST_URL is set (e.g. http://127.0.0.1:8123), run tests
    # against the Rust binary (black-box HTTP) instead of Python TestClient.
    # This is the conformance harness for the incremental Rust port.
    rust_url = os.environ.get("ANCHOR_TEST_RUST_URL")
    if rust_url:
        import httpx

        class RustClient:
            def __init__(self, base):
                self.base = base.rstrip("/")
                self.client = httpx.Client(base_url=self.base, timeout=30.0)
                # R7.1: fetch per-session CSRF token for local-mode mutating requests
                self._csrf = None
                if os.environ.get("ANCHOR_CSRF_DISABLE") != "1":
                    try:
                        r = self.client.get("/csrf", timeout=5.0)
                        if r.status_code == 200:
                            self._csrf = r.json().get("csrf_token")
                    except Exception:
                        pass

            def _csrf_headers(self, kw):
                if self._csrf and "headers" not in kw:
                    kw["headers"] = {}
                if self._csrf:
                    # httpx headers are case-insensitive; use X-CSRF-Token
                    hdrs = kw.get("headers", {})
                    # don't overwrite if already set
                    hdrs.setdefault("X-CSRF-Token", self._csrf)
                    kw["headers"] = hdrs
                return kw

            def get(self, path, **kw):
                return self.client.get(path, **kw)

            def post(self, path, **kw):
                # httpx Client.post expects json=, same as TestClient
                return self.client.post(path, **self._csrf_headers(kw))

            def put(self, path, **kw):
                return self.client.put(path, **self._csrf_headers(kw))

            def patch(self, path, **kw):
                return self.client.patch(path, **self._csrf_headers(kw))

            def delete(self, path, **kw):
                return self.client.delete(path, **self._csrf_headers(kw))

        c = RustClient(rust_url)
        yield c
        c.client.close()
        return
    from app.main import app

    with TestClient(app) as c:
        yield c


@pytest.fixture()
def isolated_db(tmp_path):
    """B38: a fresh, empty SQLite DB + session for tests that need isolation
    (e.g. onboarding-on-empty-db) without asserting against the shared test DB.

    The app's module-level engine is still bound to the shared DB, so this is
    for direct-session tests that build their own engine — NOT for `client`.
    Applies migrations (create_all covers the base schema; the app's runtime
    migration listener isn't involved here)."""
    from sqlalchemy import create_engine
    from sqlalchemy.orm import Session

    from app.db import Base

    engine = create_engine(f"sqlite:///{tmp_path / 'isolated.db'}")
    Base.metadata.create_all(engine)
    yield engine, Session(engine)
    Session(engine).close()
