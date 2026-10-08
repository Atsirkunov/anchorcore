# Hosting skeleton (v1.5 + R10.7)

> Same FastAPI app, env-driven. Local = SQLite + keychain. Hosted = Postgres + env secrets. No fork.
> **R10.7 parity decision (2026-08-22):** `hosting/` stays **Python** (FastAPI + `psycopg` + `pgvector`); Rust (`rust/target/release/anchorcore`, `9.8M`) is the **local/packaged** artifact (SQLite `WAL`, `frontend/dist` embedded). Python backend frozen on tag `archive/python-final`. Rust Postgres (deadpool + `sqlx`/`pgvector`) is deferred — would duplicate Alembic history + `vec0` triggers; hosted can alternatively run Rust binary against a SQLite file volume (`ANCHOR_DATABASE_URL=sqlite:////data/anchorcore.db`) until Postgres parity is prioritized.

## What this is
A minimal skeleton so you can run the **hosted** shape tomorrow without rewriting the app:

- `Dockerfile` — builds the backend + bundled `frontend/dist` (requires `npm run build` once).
- `docker-compose.yml` — `app` + `pgvector:pg16` Postgres, healthchecked, `pgdata` persisted. Uses `psycopg` driver.
- `.env.example` — copy to `.env` and fill `ANCHOR_*`.
- `entrypoint.sh` — `alembic upgrade head` then `uvicorn`.

Follows `docs/v2v3-scope.md:3` — same code, swap via `ANCHOR_DATABASE_URL`, `SecretStore` interface, `VectorStore` → pgvector later.

## Run hosted locally (emulation)

```bash
# from repo root
npm run build --prefix frontend   # embed UI
cp hosting/.env.example hosting/.env  # edit if needed
docker compose -f hosting/docker-compose.yml up --build
# app at http://localhost:8000
# db at localhost:5432 (anchorcore/anchorcore)
```

Migrations run via `backend/alembic/env.py` (from the archive tag) against `ANCHOR_DATABASE_URL`. Local dev still uses `data/anchorcore.db` — nothing changes unless you set `ANCHOR_DATABASE_URL` to `postgresql+...`.

## What still needs decisions (not in skeleton)

- Provider: Fly / Hetzner / Render — all work with this image. Skeleton is provider-agnostic.
- Auth: **skeleton done** (`/auth/status|signup|login|me`, B40, `ANCHOR_AUTH_SECRET` enables JWT; local stays no-auth). Next: Google OAuth / per-project share tokens (B30 gate already enforces `public_only`).
- Vector store: `vec_chunks` vec0 is SQLite-only (B33) and skips on Postgres (fallback to Python scan); Compose uses `pgvector/pgvector` image so DB is ready for future `pgvector` wiring.
- Object storage / Drive connector (B28 — next) — env keys reserved, not wired.
- TLS / domain — add Caddy/Traefik or provider's proxy.

## Personal vs Team

Personal stays local free (no license check — `ANCHOR_EDITION` unset). This
image is the Team edition (`ANCHOR_EDITION=team` baked into the Dockerfile)
and requires a one-time platform license, verified offline at startup
(`backend/app/license.py` on the archive tag — ed25519, no phone-home):

- Mount the license file: `-v /path/to/license.json:/data/license.json -e ANCHOR_LICENSE_FILE=/data/license.json`,
  or paste it inline via `-e ANCHOR_LICENSE='<json>'`.
- Missing/forged license → container refuses to boot. Expired maintenance →
  boots with a warning (perpetual license: support lapses, the app keeps running).
- Issue licenses with `python scripts/make_license.py issue --org "Acme"`.
  The signing private key lives with the seller only — never commit it.

## Next steps

1. `docker compose up` should turn green with an empty DB (smoke: `curl localhost:8000/health`).
2. Add hosted auth + read-only share links (project tokens, `docs/v2v3-scope.md:4`).
3. Wire pgvector for `vec0` → pgvector migration and bundled model keys.

Companion: `docs/architecture.md:8` (swap points), `docs/v2v3-scope.md:8` (sequencing).

See `docs/archived-python.md` for the archive layout (what's frozen, how to refresh it).
