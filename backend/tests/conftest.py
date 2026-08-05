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
# never touch the real OS keychain from tests — secrets go to the encrypted
# fallback file under the temp data dir instead
os.environ["ANCHOR_SECRETS_NO_KEYRING"] = "1"


@pytest.fixture()
def client():
    from app.main import app

    with TestClient(app) as c:
        yield c
