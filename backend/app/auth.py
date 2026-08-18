"""Minimal auth for hosted skeleton (B40).

Local: ANCHOR_AUTH_SECRET empty → auth disabled, single-user.
Hosted: set ANCHOR_AUTH_SECRET → /auth enabled (signup/login/me, Bearer JWT).

No extra deps: PBKDF2-SHA256 for passwords, HS256 JWT via hmac+base64.
"""
import base64
import hashlib
import hmac
import json
import secrets
import time
from datetime import datetime, timezone

from fastapi import Depends, HTTPException
from fastapi.security import HTTPAuthorizationCredentials, HTTPBearer
from sqlalchemy import select
from sqlalchemy.orm import Session

from .config import settings
from .db import get_db
from .models import User

_bearer = HTTPBearer(auto_error=False)

# --- password ---

_ITER = 200_000


def hash_password(password: str) -> str:
    salt = secrets.token_hex(16)
    dk = hashlib.pbkdf2_hmac("sha256", password.encode(), salt.encode(), _ITER)
    return f"pbkdf2_sha256${_ITER}${salt}${dk.hex()}"


def verify_password(password: str, stored: str) -> bool:
    try:
        _, iter_s, salt, hex_dk = stored.split("$", 3)
        it = int(iter_s)
        dk = hashlib.pbkdf2_hmac("sha256", password.encode(), salt.encode(), it)
        return hmac.compare_digest(dk.hex(), hex_dk)
    except Exception:
        return False


# --- JWT HS256 ---

def _b64url(data: bytes) -> str:
    return base64.urlsafe_b64encode(data).rstrip(b"=").decode()


def _b64url_decode(s: str) -> bytes:
    return base64.urlsafe_b64decode(s + "=" * (-len(s) % 4))


def create_token(user_id: int, email: str) -> str:
    if not settings.auth_enabled:
        raise RuntimeError("auth not enabled")
    header = _b64url(json.dumps({"alg": "HS256", "typ": "JWT"}).encode())
    exp = int(time.time()) + settings.auth_token_hours * 3600
    payload = _b64url(json.dumps({"sub": user_id, "email": email, "exp": exp}).encode())
    sig = _b64url(hmac.new(settings.auth_secret.encode(), f"{header}.{payload}".encode(), hashlib.sha256).digest())
    return f"{header}.{payload}.{sig}"


def decode_token(token: str) -> dict:
    try:
        h, p, s = token.split(".")
        expected = _b64url(hmac.new(settings.auth_secret.encode(), f"{h}.{p}".encode(), hashlib.sha256).digest())
        if not hmac.compare_digest(s, expected):
            raise ValueError("bad signature")
        data = json.loads(_b64url_decode(p))
        if data.get("exp", 0) < time.time():
            raise ValueError("expired")
        return data
    except Exception as exc:
        raise ValueError(str(exc)) from exc


def get_current_user(
    cred: HTTPAuthorizationCredentials | None = Depends(_bearer),
    db: Session = Depends(get_db),
) -> User | None:
    if not settings.auth_enabled:
        return None
    if cred is None:
        raise HTTPException(status_code=401, detail="Not authenticated")
    try:
        data = decode_token(cred.credentials)
    except ValueError as exc:
        raise HTTPException(status_code=401, detail=str(exc)) from exc
    user = db.get(User, int(data["sub"]))
    if user is None:
        raise HTTPException(status_code=401, detail="User not found")
    return user


def require_auth(user: User | None = Depends(get_current_user)) -> User:
    # When auth disabled, allow single-user local access (return dummy None allowed by callers).
    # Protected routes should call `require_auth` only when they truly need auth.
    if not settings.auth_enabled:
        # local mode: fabricate a synthetic local user so callers have something
        return User(id=0, email="local", password_hash="", created_at=datetime.now(timezone.utc))  # type: ignore
    if user is None:
        raise HTTPException(status_code=401, detail="Not authenticated")
    return user
