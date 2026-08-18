#!/bin/sh
set -eu
echo "--- AnchorCore hosted entrypoint ---"
echo "DB: ${ANCHOR_DATABASE_URL:-sqlite:////data/anchorcore.db}"
echo "CORS: ${ANCHOR_CORS_ORIGINS:-*}"
# Migrations also run on app startup (main.py lifespan); this pre-migrate is best-effort
# so the first boot doesn't have to wait for the app to do it. Use correct ini path.
python -m alembic -c backend/alembic.ini upgrade head || echo "pre-migrate failed — app will retry on startup"
exec uvicorn app.main:app --host 0.0.0.0 --port 8000 --app-dir backend
