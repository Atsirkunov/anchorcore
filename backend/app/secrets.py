import hashlib
import json
import logging
import os
from pathlib import Path

from cryptography.fernet import Fernet, InvalidToken
from sqlalchemy.orm import Session

from .models import Source

logger = logging.getLogger(__name__)

# B34: single source of truth for which source-config fields hold secrets. Both
# SecretStore.store_source_config and the sources router mask/route these;
# adding a new secret field must go here, not in both places.
SECRET_SOURCE_FIELDS = ("token", "api_key", "password")


class SecretStore:
    """Credentials in the OS keychain; encrypted-file fallback when unavailable.

    Set ANCHOR_SECRETS_NO_KEYRING=1 to force the encrypted-file fallback
    (used by tests so they never touch the real OS keychain).

    The fallback is a per-key set of encrypted files under the data dir (one
    file per key), NOT a single blob — `delete(key)` must only remove that
    key's secret (the original single-file design wiped every stored secret
    on any delete).
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

    def _fallback_file(self, key: str) -> Path:
        digest = hashlib.sha256(key.encode("utf-8")).hexdigest()[:32]
        return self.fallback_path.with_name(f"{self.fallback_path.name}.{digest}")

    def set(self, key: str, value: str) -> None:
        if self._use_keyring:
            try:
                import keyring

                keyring.set_password(self.SERVICE, key, value)
                return
            except Exception as exc:  # noqa: BLE001
                logger.warning("keyring unavailable (%s); using encrypted fallback file", exc)
        path = self._fallback_file(key)
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(self._fernet.encrypt(value.encode()))

    def get(self, key: str) -> str | None:
        if self._use_keyring:
            try:
                import keyring

                value = keyring.get_password(self.SERVICE, key)
                if value is not None:
                    return value
            except Exception:  # noqa: BLE001
                pass
        path = self._fallback_file(key)
        if not path.exists():
            return None
        try:
            return self._fernet.decrypt(path.read_bytes()).decode()
        except InvalidToken:
            return None

    def delete(self, key: str) -> None:
        if self._use_keyring:
            try:
                import keyring

                keyring.delete_password(self.SERVICE, key)
            except Exception:  # noqa: BLE001
                pass
        try:
            self._fallback_file(key).unlink()
        except OSError:
            pass


def source_secret_key(source: Source, field: str) -> str:
    return f"source:{source.id}:{field}"


def store_source_config(
    db: Session, source: Source, config: dict, secrets: SecretStore
) -> None:
    """Persist config, moving secret values into the SecretStore.

    The '***set***' sentinel (sent by the UI for a field whose stored value
    should stay) is skipped; an absent field is left untouched; an explicitly
    empty string clears the stored secret."""
    safe = dict(config)
    for field in SECRET_SOURCE_FIELDS:
        value = safe.pop(field, None)
        if value is None or value == "***set***":
            continue  # not provided / UI placeholder — keep the stored secret
        if value:
            secrets.set(source_secret_key(source, field), str(value))
        else:
            secrets.delete(source_secret_key(source, field))
    source.config = json.dumps(safe)


def resolve_source_config(source: Source, secrets: SecretStore) -> dict:
    """Rehydrate config with secrets from the SecretStore."""
    config = source.config_dict()
    for field in SECRET_SOURCE_FIELDS:
        value = secrets.get(source_secret_key(source, field))
        if value is not None:
            config[field] = value
    return config
