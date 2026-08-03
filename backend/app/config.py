from pathlib import Path

from pydantic_settings import BaseSettings, SettingsConfigDict


class Settings(BaseSettings):
    model_config = SettingsConfigDict(env_file=".env", env_prefix="ANCHOR_")

    data_dir: Path = Path("data")
    database_url: str = ""
    cors_origins: str = "http://localhost:5173"

    ollama_base_url: str = "http://localhost:11434"
    classifier_model: str = "llama3.2:3b"
    embed_model: str = "nomic-embed-text"
    classifier_timeout: float = 60.0

    answer_model: str = "gpt-4o-mini"
    answer_api_key: str = ""
    answer_base_url: str = "https://api.openai.com/v1"
    answer_timeout: float = 90.0

    jira_poll_minutes: int = 15
    folder_scan_minutes: int = 60
    folder_watch_debounce: float = 3.0
    chunk_size: int = 800
    chunk_overlap: int = 100
    top_k: int = 8
    low_confidence_threshold: float = 0.6
    duplicate_threshold: float = 0.92

    @property
    def resolved_database_url(self) -> str:
        return self.database_url or f"sqlite:///{self.data_dir / 'anchorcore.db'}"


settings = Settings()
