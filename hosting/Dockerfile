# Hosting image — same FastAPI app, env-driven.
# Build: docker build -f hosting/Dockerfile -t anchorcore:hosting .
FROM python:3.12-slim

WORKDIR /app

RUN apt-get update && apt-get install -y --no-install-recommends \
    libsqlite3-0 curl \
  && rm -rf /var/lib/apt/lists/*

COPY backend/requirements.txt ./requirements.txt
RUN pip install --no-cache-dir -r requirements.txt

# psycopg for Postgres (hosted DB). Not needed locally.
RUN pip install --no-cache-dir "psycopg[binary]==3.2.2" "pgvector==0.3.6"

COPY backend ./backend
COPY frontend/dist ./frontend/dist
COPY hosting/entrypoint.sh ./entrypoint.sh
RUN chmod +x ./entrypoint.sh

ENV ANCHOR_DATA_DIR=/data
ENV PYTHONPATH=/app/backend
# This image is the Team (self-hosted) edition — requires a platform license
# (backend/app/license.py). Personal local runs never set ANCHOR_EDITION.
ENV ANCHOR_EDITION=team

EXPOSE 8000
CMD ["./entrypoint.sh"]
