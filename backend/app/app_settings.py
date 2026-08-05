"""Runtime-mutable settings (B4): DB-backed overrides + SecretStore keys.

`Settings` (env/.env) stays the default layer. `SettingsService` resolves the
*effective* value per key: secrets from the SecretStore, then DB `app_settings`
overrides, then env defaults. Consumers read through the service at call time,
so changes apply without restart. A short TTL cache keeps hot paths (classify
windows, embeddings) off the DB; writes invalidate it.
"""

import logging
import threading
import time

from sqlalchemy import select
from sqlalchemy.orm import Session

from .config import Settings
from .db import SessionLocal
from .models import AppSetting
from .secrets import SecretStore

logger = logging.getLogger(__name__)

# Keys the UI exposes (B4). Secrets live in the SecretStore, never in DB.
SETTING_KEYS = (
    "ollama_base_url",
    "classifier_model",
    "embed_model",
    "classifier_timeout",
    "answer_model",
    "answer_base_url",
    "answer_timeout",
)
SECRET_KEYS = ("answer_api_key",)
ALL_KEYS = SETTING_KEYS + SECRET_KEYS

SECRET_PREFIX = "app:"
CACHE_TTL_SECONDS = 3.0


class SettingsService:
    def __init__(self, env: Settings, secrets: SecretStore):
        self.env = env
        self.secrets = secrets
        self._cache: dict[str, tuple[str, float]] = {}
        self._lock = threading.Lock()

    # ---- resolution -------------------------------------------------

    def get(self, key: str, db: Session | None = None) -> str | None:
        if key not in ALL_KEYS:
            return getattr(self.env, key, None) or None
        if key in SECRET_KEYS:
            secret = self.secrets.get(f"{SECRET_PREFIX}{key}")
            if secret is not None:
                return secret
            return self._env_value(key)
        if db is not None:
            value = self._db_value(db, key)
            if value is not None:
                return value
            return self._env_value(key)
        return self._cached_get(key)

    def get_float(self, key: str, default: float = 0.0) -> float:
        try:
            raw = self.get(key)
            return float(raw) if raw else default
        except (TypeError, ValueError):
            return default

    def snapshot(self, db: Session | None = None) -> dict:
        """Effective values for every setting; secrets masked."""
        out = {}
        for key in ALL_KEYS:
            value = self.get(key, db=db)
            if key in SECRET_KEYS:
                out[key] = "***set***" if value else ""
            else:
                out[key] = value if value is not None else ""
        return out

    # ---- writes -----------------------------------------------------

    def set(self, key: str, value: str, db: Session | None = None) -> None:
        value = (value or "").strip()
        if key not in ALL_KEYS:
            logger.warning("ignoring unknown setting key '%s'", key)
            return
        if key in SECRET_KEYS:
            if value:
                self.secrets.set(f"{SECRET_PREFIX}{key}", value)
            else:
                self.secrets.delete(f"{SECRET_PREFIX}{key}")
            self._invalidate(key)
            logger.info("setting %s updated (secret)", key)
            return
        with db or SessionLocal() as session:
            row = session.get(AppSetting, key)
            if row is None:
                session.add(AppSetting(key=key, value=value))
            else:
                row.value = value
            session.commit()
        self._invalidate(key)
        logger.info("setting %s updated", key)

    def clear(self, key: str, db: Session | None = None) -> None:
        """Restore env default (remove DB override / stored secret)."""
        if key not in ALL_KEYS:
            return
        if key in SECRET_KEYS:
            self.secrets.delete(f"{SECRET_PREFIX}{key}")
        else:
            with db or SessionLocal() as session:
                row = session.get(AppSetting, key)
                if row is not None:
                    session.delete(row)
                    session.commit()
        self._invalidate(key)

    # ---- internals --------------------------------------------------

    def _env_value(self, key: str) -> str | None:
        value = getattr(self.env, key, None)
        return value if value is not None and value != "" else None

    def _db_value(self, db: Session, key: str) -> str | None:
        row = db.get(AppSetting, key)
        return row.value if row is not None else None

    def _cached_get(self, key: str) -> str | None:
        now = time.monotonic()
        with self._lock:
            cached = self._cache.get(key)
            if cached is not None and cached[1] > now:
                return cached[0] or None
        with SessionLocal() as db:
            value = self._db_value(db, key)
        resolved = value if value is not None else self._env_value(key)
        with self._lock:
            self._cache[key] = (resolved or "", now + CACHE_TTL_SECONDS)
        return resolved

    def _invalidate(self, key: str) -> None:
        with self._lock:
            self._cache.pop(key, None)


def settings_service() -> SettingsService:
    """Convenience for module-level wiring in tests/scripts."""
    from .config import settings as env_settings

    return SettingsService(env_settings, SecretStore(env_settings.data_dir / "secrets.enc"))
