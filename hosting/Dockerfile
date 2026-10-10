# syntax=docker/dockerfile:1
# AnchorCore Team image — ONE container running the same Rust backend as the
# local app (R19.1 / B59). Frontend is embedded into the binary at build time;
# state lives in a SQLite file on the /data volume. No Python, no Postgres.
#
#   docker build -f hosting/Dockerfile -t anchorcore:team .
#
# The image bakes ANCHOR_EDITION=team and refuses to boot without a valid
# license (see hosting/README.md). For a bare smoke, override the env:
#   docker run -e ANCHOR_EDITION=personal -p 8000:8000 anchorcore:team

# ---------- 1) frontend (embedded into the binary via include_dir!) ----------
FROM node:20-slim AS frontend
WORKDIR /src/frontend
COPY frontend/package.json frontend/package-lock.json ./
RUN npm ci --no-audit --no-fund
COPY frontend/ ./
RUN npm run build

# ---------- 2) Rust release binary (same build as the release zips) ----------
FROM rust:1-slim-bookworm AS builder
RUN apt-get update \
  && apt-get install -y --no-install-recommends build-essential pkg-config \
  && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY rust/ rust/
COPY --from=frontend /src/frontend/dist frontend/dist
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/rust/target \
    cargo build --release -p anchorcore --manifest-path rust/Cargo.toml \
  && cp rust/target/release/anchorcore /usr/local/bin/anchorcore

# ---------- 3) runtime — one process, one volume ----------
FROM debian:bookworm-slim
RUN apt-get update \
  && apt-get install -y --no-install-recommends ca-certificates curl \
  && rm -rf /var/lib/apt/lists/* \
  && useradd --system --uid 10001 --create-home --home-dir /home/anchorcore anchorcore \
  && install -d -o anchorcore -g anchorcore /data
COPY --from=builder /usr/local/bin/anchorcore /usr/local/bin/anchorcore
ENV ANCHOR_DATA_DIR=/data \
    ANCHOR_SECRETS_NO_KEYRING=1 \
    ANCHOR_EDITION=team \
    ANCHOR_OPEN_BROWSER=0
USER anchorcore
WORKDIR /home/anchorcore
EXPOSE 8000
HEALTHCHECK --interval=30s --timeout=5s --start-period=30s --retries=3 \
  CMD curl -fsS http://127.0.0.1:8000/health || exit 1
CMD ["anchorcore", "--host", "0.0.0.0", "--port", "8000", "--data-dir", "/data"]
