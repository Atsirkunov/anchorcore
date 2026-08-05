import json
import logging
import os
from pathlib import Path

from cryptography.fernet import Fernet, InvalidToken
from sqlalchemy.orm import Session

from .models import Source

logger = logging.getLogger(__name__)


class SecretStore:
    """Credentials in the OS keychain; encrypted-file fallback when unavailable.

    Set ANCHOR_SECRETS_NO_KEYRING=1 to force the encrypted-file fallback
    (used by tests so they never touch the real OS keychain).
    """

    SERVICE = "AnchorCore"

    def __init__(self, fallback_path: Path):
        self.fallback_path = fallback_path
        self._use_keyring = not (os.environ.get("ANCHOR_SECRETS_NO_KEYRING") == "1")
        self._fernet = self._load_fernet()

    def _load_fernet(self) -> Fernet:
        key_path = self.fallback_path.with_suffix(".key")
        if key_path.exists():
            return Fernet(key_path.read_text().strip())
        key = Fernet.generate_key()
        key_path.parent.mkdir(parents=True, exist_ok=True)
        key_path.write_text(key.decode())
        try:
            key_path.chmod(0o600)
        except OSError:
            pass
        return Fernet(key)

    def set(self, key: str, value: str) -> None:
        if self._use_keyring:
            try:
                import keyring

                keyring.set_password(self.SERVICE, key, value)
                return
            except Exception as exc:  # noqa: BLE001
                logger.warning("keyring unavailable (%s); using encrypted fallback file", exc)
        encrypted = self._fernet.encrypt(value.encode())
        self.fallback_path.parent.mkdir(parents=True, exist_ok=True)
        self.fallback_path.write_bytes(encrypted)

    def get(self, key: str) -> str | None:
        if self._use_keyring:
            try:
                import keyring

                value = keyring.get_password(self.SERVICE, key)
                if value is not None:
                    return value
            except Exception:  # noqa: BLE001
                pass
        if not self.fallback_path.exists():
            return None
        try:
            return self._fernet.decrypt(self.fallback_path.read_bytes()).decode()
        except InvalidToken:
            return None

    def delete(self, key: str) -> None:
        if self._use_keyring:
            try:
                import keyring

                keyring.delete_password(self.SERVICE, key)
            except Exception:  # noqa: BLE001
                pass
        if self.fallback_path.exists():
            try:
                self.fallback_path.unlink()
            except OSError:
                pass


def source_secret_key(source: Source, field: str) -> str:
    return f"source:{source.id}:{field}"


def store_source_config(
    db: Session, source: Source, config: dict, secrets: SecretStore
) -> None:
    """Persist config, moving secret values into the SecretStore."""
    safe = dict(config)
    for field in ("token", "api_key", "password"):
        value = safe.pop(field, None)
        if value:
            secrets.set(source_secret_key(source, field), str(value))
    source.config = json.dumps(safe)


def resolve_source_config(source: Source, secrets: SecretStore) -> dict:
    """Rehydrate config with secrets from the SecretStore."""
    config = source.config_dict()
    for field in ("token", "api_key", "password"):
        value = secrets.get(source_secret_key(source, field))
        if value is not None:
            config[field] = value
    return config
