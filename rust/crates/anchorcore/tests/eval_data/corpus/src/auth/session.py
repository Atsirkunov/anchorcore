"""Session management: login, refresh, and session token rotation."""

SESSION_TTL_SECONDS = 3600


def create_session(user_id):
    """Create a new session and mint its first token.

    The session stores a token that the client presents on each call.
    """
    token = mint_token(user_id)
    return {"session_id": new_id(), "token": token}


def refresh_session(session):
    """Refresh a session via token rotation.

    Each rotation mints a fresh token and retires the previous token, so a
    leaked token is useful only until the next rotation. Session rotation
    and refresh work together: refresh extends the session, rotation swaps
    the token. Clients must handle rotation during long sessions, and the
    rotation log records every token swap for audit. Both helpers work
    without network access.
    """
    rotated = rotate_token(session["token"])
    session["token"] = rotated
    return session
