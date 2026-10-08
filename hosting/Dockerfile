# Hosting image — frozen Python backend + Team license gate, env-driven.
# Build from the repo root: docker build -f hosting/Dockerfile -t anchorcore:hosting .
# backend/ was archived on git tag archive/python-final (frozen); the image
# fetches that snapshot at build time. Move BACKEND_TAG only for license-gate fixes.
FROM python:3.12-slim

ARG BACKEND_TAG=archive/python-final

WORKDIR /app

# psycopg is for Postgres (hosted DB). Not needed locally.
RUN apt-get update && apt-get install -y --no-install-recommends \
    libsqlite3-0 curl \
  && rm -rf /var/lib/apt/lists/* \
  && curl -sL "https://github.com/Atsirkunov/anchorcore/archive/refs/tags/${BACKEND_TAG}.tar.gz" -o /tmp/py.tar.gz \
  && mkdir -p /tmp/py && tar -xzf /tmp/py.tar.gz -C /tmp/py --strip-components=1 \
  && cp /tmp/py/backend/requirements.txt ./requirements.txt \
  && pip install --no-cache-dir -r requirements.txt \
  && pip install --no-cache-dir "psycopg[binary]==3.2.2" "pgvector==0.3.6" \
  && cp -r /tmp/py/backend ./backend \
  && rm -rf /tmp/py /tmp/py.tar.gz

COPY frontend/dist ./frontend/dist
COPY hosting/entrypoint.sh ./entrypoint.sh
RUN chmod +x ./entrypoint.sh

ENV ANCHOR_DATA_DIR=/data
ENV PYTHONPATH=/app/backend
# This image is the Team (self-hosted) edition — requires a platform license
# (backend/app/license.py, frozen on the archive tag). Personal local runs never set ANCHOR_EDITION.
ENV ANCHOR_EDITION=team

EXPOSE 8000
CMD ["./entrypoint.sh"]
