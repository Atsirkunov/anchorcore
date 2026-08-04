import logging
from datetime import datetime, timezone

from sqlalchemy import select
from sqlalchemy.orm import Session

from .db import SessionLocal
from .models import SystemEvent
from .redact import redact

logger = logging.getLogger(__name__)

MAX_EVENTS = 500


def record(
    component: str,
    message: str,
    level: str = "error",
    source_id: int | None = None,
    detail: str = "",
    db: Session | None = None,
) -> None:
    """Persist a structured system event (redacted).

    Best-effort and NEVER raises: error recording must not break the code
    path that failed in the first place. When the provided session is in a
    bad state (e.g. after an exception), falls back to its own session."""
    event = SystemEvent(
        component=component,
        level=level,
        source_id=source_id,
        message=redact(str(message))[:2000],
        detail=redact(str(detail))[:4000],
        created_at=datetime.now(timezone.utc),
    )
    if db is not None:
        try:
            _persist(db, event)
            return
        except Exception:  # noqa: BLE001
            try:
                db.rollback()
            except Exception:  # noqa: BLE001
                pass
    try:
        with SessionLocal() as db:
            _persist(db, event)
    except Exception as exc:  # noqa: BLE001
        logger.error("failed to record system event: %s", exc)


def _persist(db: Session, event: SystemEvent) -> None:
    db.add(event)
    db.flush()
    stale = (
        db.execute(
            select(SystemEvent.id).order_by(SystemEvent.created_at.desc()).offset(MAX_EVENTS)
        )
        .scalars()
        .all()
    )
    for event_id in stale:
        db.delete(db.get(SystemEvent, event_id))
    db.commit()
