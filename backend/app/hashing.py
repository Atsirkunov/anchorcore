"""Hashing helpers (B35): content + window hashes.

Single source of truth for the sha256-based hashes used to skip unchanged
documents/windows on re-sync. `window_hash` and `content_hash` were previously
duplicated across classifier.py / models.py — import them from here.
"""

import hashlib

__all__ = ["window_hash", "content_hash"]


def window_hash(text: str) -> str:
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def content_hash(text: str) -> str:
    return hashlib.sha256(text.encode("utf-8")).hexdigest()
