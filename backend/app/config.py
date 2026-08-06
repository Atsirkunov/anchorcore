import logging
import sys
from pathlib import Path

from pydantic_settings import BaseSettings, SettingsConfigDict

logger = logging.getLogger(__name__)


def _env_file_path() -> Path:
    return Path(".env")


def _default_data_dir() -> Path:
    """Frozen apps (PyInstaller) keep data per-user, outside the exe dir."""
    if getattr(sys, "frozen", False):
        return Path.home() / ".anchorcore"
    return Path("data")


def validate_env_file() -> list[str]:
    """Check the .env file for duplicate keys and typos (unknown ANCHOR_* keys).

    Returns a list of human-readable warnings; empty when the file is clean.
    """
    env_path = _env_file_path()
    if not env_path.exists():
        return []
    warnings: list[str] = []
    seen: dict[str, int] = {}
    for lineno, raw in enumerate(env_path.read_text(encoding="utf-8").splitlines(), start=1):
        line = raw.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key = line.split("=", 1)[0].strip()
        if not key:
            continue
        if key in seen:
            warnings.append(f".env:{lineno}: duplicate key '{key}' (first defined on line {seen[key]}) — last value wins")
        seen[key] = lineno
        if key.startswith("ANCHOR_") and key.removeprefix("ANCHOR_").lower() not in Settings.model_fields:
            warnings.append(f".env:{lineno}: unknown key '{key}' — is it a typo? pydantic-settings ignores it")
    return warnings


class Settings(BaseSettings):
    model_config = SettingsConfigDict(env_file=".env", env_prefix="ANCHOR_", extra="ignore")

    data_dir: Path = Path(_default_data_dir())
    database_url: str = ""
    cors_origins: str = "http://localhost:5173"

    ollama_base_url: str = "http://localhost:11434"
    classifier_model: str = "llama3.2:3b"
    classifier_base_url: str = ""  # empty → ollama_base_url (local)
    classifier_api_key: str = ""
    embed_model: str = "nomic-embed-text"
    classifier_timeout: float = 60.0
    classifier_concurrency: int = 4
    http_retries: int = 3

    answer_model: str = "gpt-4o-mini"
    answer_api_key: str = ""
    answer_base_url: str = "https://api.openai.com/v1"
    answer_timeout: float = 90.0
    answer_reasoning_effort: str = "none"

    jira_poll_minutes: int = 15
    folder_scan_minutes: int = 60
    folder_watch_debounce: float = 3.0
    chunk_size: int = 800
    chunk_overlap: int = 100
    chunk_max_chars: int = 1600
    classify_window_chars: int = 8000
    retrieval_keyword_weight: float = 1.0
    retrieval_max_per_source: int = 3
    retrieval_age_halflife_days: int = 365
    retrieval_context_window: int = 1
    top_k: int = 8
    low_confidence_threshold: float = 0.6
    duplicate_threshold: float = 0.92

    @property
    def resolved_database_url(self) -> str:
        return self.database_url or f"sqlite:///{self.data_dir / 'anchorcore.db'}"

    def banner(self) -> list[str]:
        """Effective settings shown at boot so a fresh checkout is easy to sanity-check."""
        if self.answer_base_url.startswith(("http://localhost", "http://127.0.0.1")):
            answer_provider = f"ollama ({self.answer_model})"
        elif self.answer_api_key:
            answer_provider = f"{self.answer_base_url} ({self.answer_model})"
        else:
            answer_provider = f"{self.answer_base_url} — API key MISSING (answers fall back to context only)"
        return [
            "--- AnchorCore config ---",
            f"  data dir        : {self.data_dir}",
            f"  database        : {self.resolved_database_url}",
            f"  ollama          : {self.ollama_base_url}",
            f"  classifier model: {self.classifier_model}",
            f"  classifier conc : {self.classifier_concurrency}",
            f"  embed model     : {self.embed_model}",
            f"  retrieval       : RRF fusion (keyword weight {self.retrieval_keyword_weight}, "
            f"max {self.retrieval_max_per_source}/source, age halflife {self.retrieval_age_halflife_days}d)",
            f"  answer model    : {answer_provider}",
            "-------------------------",
        ]


settings = Settings()
