from pathlib import Path

from pydantic_settings import BaseSettings, SettingsConfigDict


class Settings(BaseSettings):
    model_config = SettingsConfigDict(env_file=".env", env_prefix="ANCHOR_")

    database_url: str = "sqlite:///./anchorcore.db"
    cors_origins: str = "http://localhost:3000"

    classifier_base_url: str = "http://localhost:11434/v1"
    classifier_model: str = "llama3.2"
    classifier_api_key: str = "ollama"
    classifier_timeout: float = 60.0

    upload_dir: Path = Path("uploads")


settings = Settings()
