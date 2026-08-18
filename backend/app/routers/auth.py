from fastapi import APIRouter, Depends, HTTPException
from pydantic import BaseModel, field_validator
from sqlalchemy import select
from sqlalchemy.orm import Session

from ..auth import create_token, hash_password, verify_password, get_current_user
from ..config import settings
from ..db import get_db
from ..models import User


def _valid_email(v: str) -> str:
    v = v.strip().lower()
    if "@" not in v or "." not in v.split("@")[-1]:
        raise ValueError("invalid email")
    return v


class SignupIn(BaseModel):
    email: str
    password: str

    @field_validator("email")
    @classmethod
    def _email(cls, v: str) -> str:
        return _valid_email(v)


class LoginIn(BaseModel):
    email: str
    password: str

    @field_validator("email")
    @classmethod
    def _email(cls, v: str) -> str:
        return _valid_email(v)


def make_router() -> APIRouter:
    router = APIRouter(prefix="/auth", tags=["auth"])

    @router.get("/status")
    def status():
        return {"enabled": settings.auth_enabled}

    @router.post("/signup", status_code=201)
    def signup(payload: SignupIn, db: Session = Depends(get_db)):
        if not settings.auth_enabled:
            raise HTTPException(status_code=404, detail="Auth disabled (set ANCHOR_AUTH_SECRET)")
        if len(payload.password) < 8:
            raise HTTPException(status_code=422, detail="Password must be >=8 chars")
        exists = db.execute(select(User).where(User.email == payload.email.lower())).scalar_one_or_none()
        if exists:
            raise HTTPException(status_code=409, detail="Email already registered")
        user = User(email=payload.email.lower(), password_hash=hash_password(payload.password))
        db.add(user)
        db.commit()
        db.refresh(user)
        token = create_token(user.id, user.email)
        return {"access_token": token, "token_type": "bearer", "user": {"id": user.id, "email": user.email}}

    @router.post("/login")
    def login(payload: LoginIn, db: Session = Depends(get_db)):
        if not settings.auth_enabled:
            raise HTTPException(status_code=404, detail="Auth disabled")
        user = db.execute(select(User).where(User.email == payload.email.lower())).scalar_one_or_none()
        if not user or not verify_password(payload.password, user.password_hash):
            raise HTTPException(status_code=401, detail="Invalid credentials")
        token = create_token(user.id, user.email)
        return {"access_token": token, "token_type": "bearer", "user": {"id": user.id, "email": user.email}}

    @router.get("/me")
    def me(user: User = Depends(get_current_user)):
        if not settings.auth_enabled:
            raise HTTPException(status_code=404, detail="Auth disabled")
        # get_current_user already 401s when missing/invalid
        return {"id": user.id, "email": user.email}

    return router
