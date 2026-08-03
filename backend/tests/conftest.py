import os
import tempfile
from pathlib import Path

import pytest
from fastapi.testclient import TestClient

_db_path = Path(tempfile.mkdtemp(prefix="anchorcore-")) / "test.db"

os.environ["ANCHOR_DATABASE_URL"] = f"sqlite:///{_db_path}"
os.environ["ANCHOR_OLLAMA_BASE_URL"] = "http://localhost:1"
os.environ["ANCHOR_CLASSIFIER_TIMEOUT"] = "1.0"
os.environ["ANCHOR_HTTP_RETRIES"] = "0"
os.environ["ANCHOR_ANSWER_BASE_URL"] = "https://api.openai.com/v1"
os.environ["ANCHOR_DATA_DIR"] = str(Path(tempfile.mkdtemp(prefix="anchorcore-data-")))


@pytest.fixture()
def client():
    from app.main import app

    with TestClient(app) as c:
        yield c
