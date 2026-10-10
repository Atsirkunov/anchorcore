# Team self-hosted — one container (Rust)

> **Same backend as the Personal app.** This image runs the shipped Rust
> binary: Axum + SQLite (sqlite-vec/FTS5), the React UI embedded, state on a
> `/data` volume. One container, one process — no Postgres sidecar, no Python.
> The Team edition requires a one-time platform license, verified offline at
> startup (missing/forged refuses to boot; lapsed maintenance warns and runs).

## Quickstart

```bash
# from the repo root
cp hosting/.env.example hosting/.env     # edit: license + models + access URL
docker compose -f hosting/docker-compose.yml up --build
# app at http://localhost:8000
```

Build without Compose:

```bash
docker build -f hosting/Dockerfile -t anchorcore:team .
docker run -d --name anchorcore -p 8000:8000 \
  -v anchorcore-data:/data \
  -v "$PWD/license.json:/data/license.json:ro" \
  -e ANCHOR_LICENSE_FILE=/data/license.json \
  -e ANCHOR_OLLAMA_BASE_URL=http://host.docker.internal:11434 \
  anchorcore:team
```

Releases also publish a prebuilt image: `ghcr.io/atsirkunov/anchorcore:<version>`
and `:team` (built by CI on `v*` tags).

## License

The image bakes `ANCHOR_EDITION=team` and verifies the license offline on
every boot — ed25519, format-compatible with `scripts/make_license.py`
(checker: `rust/crates/anchorcore/src/license.rs`).

- Mount the file: `-v /path/to/license.json:/data/license.json:ro -e ANCHOR_LICENSE_FILE=/data/license.json`
  (Compose: uncomment the volume + env lines in `docker-compose.yml`), or paste
  the JSON into `ANCHOR_LICENSE`.
- Missing/forged → refuses to boot (exit 2). Expired maintenance → boots with a
  warning (perpetual license: support/updates lapse, the app keeps running).
- Issue licenses with `python scripts/make_license.py issue --org "Acme"` on
  the seller machine. The signing private key never ships or gets committed.

## Models

Same options as Personal:

- **Local (default):** run [Ollama](https://ollama.com) on the host; Compose
  already points the container at `host.docker.internal:11434`. Pull
  `llama3.2:3b` + `nomic-embed-text` there.
- **Cloud:** any OpenAI-compatible API — set base URL + key in `.env` or in
  the app's Settings tab (runtime-mutable, no restart).
- **Without models:** keyword search still works; answers degrade honestly.

## Networking & auth

- The container listens on `0.0.0.0:8000`; map ports or put it behind your
  ingress / Cloudflare Tunnel. The app speaks plain HTTP — terminate TLS in
  front (Caddy, Traefik, your cloud LB).
- `http://localhost:8000` works out of the box. For LAN/VPC IPs or a domain,
  add each access URL to `ANCHOR_CORS_ORIGINS` (Host/Origin allowlist).
- Team logins: set `ANCHOR_AUTH_SECRET` to enable `/auth` (signup/login,
  HS256 JWT). Unset = single-user, no login — keep the port private then.

## Operations

- **Data** (the `appdata` volume, `/data`): `anchorcore.db` (SQLite),
  `secrets.enc.*` (file secret store), `anchorcore.log`, your `license.json`
  if mounted. Back it up like any file — there is no database server.
- **Migrations** apply automatically at startup (`rusqlite_migration`).
- **Health:** `GET /health`; the image ships a `HEALTHCHECK`.
- **Config:** `.env` / compose `environment:` — see `.env.example`. Runtime
  model settings can also be changed in the app (Settings tab overrides env).

## History (why this changed)

The first skeleton (B46/v1.5) was a two-service Python stack — FastAPI built
from the frozen `archive/python-final` snapshot + a `pgvector/pg16` Postgres —
while the website promised "One Docker container" (B59). R19.1 moved the Team
image onto the shipped Rust binary: the promise is now the artifact. Postgres +
pgvector remains an option for the future Hosted (cloud) tier only; it is not
the Team self-hosted story.

See `docs/archived-python.md` for the archive layout (frozen backend, used by
the conformance suite and nothing else).
