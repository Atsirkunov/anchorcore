# AnchorCore — Hosted Infra Plan (v2 + R10.7)

> Locked 2026-08-10 (decisions D8–D9 in [v2v3-scope.md](./v2v3-scope.md)). Operational
> reference for the hosted tier (B35). One-box topology until real demand; no
> K8s/multi-region/autoscaling.
> **R10.7 (2026-08-22):** `hosting/` stays **Python** (FastAPI `psycopg`/`pgvector`); Rust `rust/` is **local/packaged** (`cargo build --release` `9.8M` SQLite + `frontend/dist`). Rust Postgres (`deadpool` + `sqlx`) deferred; if needed, Rust can run hosted against a SQLite volume (`sqlite:////data/...`) — no code fork.

## Topology

```
anchorcore.dev
  └── Cloudflare (free): DNS, TLS, CDN, DDoS absorption
        └── Cloudflare Tunnel (cloudflared) — egress-only ingress; no open ports
              └── Hetzner CX22 (2 vCPU / 4 GB / 40 GB, ~€4–5/mo)
                    └── Docker Compose
                          ├── anchorcore  (Python FastAPI — **hosting stays Python**, see R10.7)
                          └── postgres    (replaces SQLite; DB swap point — Rust is local SQLite only)
  Files  → Cloudflare R2 (S3-compatible; free 10 GB / 1M Class A / 10M Class B ops/mo; $0 egress)
  Billing → Stripe Checkout (hosted pages) + payment webhooks → feature unlock
  Uptime → UptimeRobot (free) → email on failure
```

## Components

| Piece | Choice | Notes |
|---|---|---|
| DNS/TLS/CDN | Cloudflare free plan | certs automatic; TLS terminates at edge |
| Ingress | Cloudflare Tunnel | no exposed ports, no firewall config; tunnel dials out |
| Compute | Hetzner CX22 | 4 GB RAM headroom for Postgres + app |
| Runtime | Docker Compose | single deploy unit |
| DB | PostgreSQL 16 (official image) | env-driven swap; SQLite stays local |
| Vector store | pgvector / Qdrant (defer) | VectorStore swap point; sqlite-vec fine at pilot |
| Object storage | Cloudflare R2 | bucket per workspace; $0 egress → share-link serving |
| Billing | Team self-hosted: offline license files (`hosting/README.md`); hosted (later): Stripe Checkout + webhooks | no license server — sign with `scripts/make_license.py`, verify offline |
| Monitoring | UptimeRobot HTTP ping on `/health` | 5-min cadence; alert email |
| Logs | existing `system_events` + file logs | `GET /system/logs` download; rotate on VPS disk |
| Backups | nightly `pg_dump` → R2 (versioned) | RPO 24h; restore doc below |
| Secrets | `.env` on VPS only (never in repo) | app secrets via existing SecretStore contract |

## Provisioning (one-time, ~1h)

1. Cloudflare: add `anchorcore.dev` zone; proxy DNS.
2. Hetzner: create CX22 (Ubuntu LTS); install Docker + compose plugin.
3. `cloudflared tunnel login` → create tunnel → route `anchorcore.dev` →
   `http://localhost:8000`; run as systemd service.
4. `docker compose up -d` with env: `POSTGRES_*`, `ANCHOR_SECRETS_*`, model keys,
   R2 credentials (S3-compatible client), `STRIPE_*` webhook secret.
5. Alembic migrations run in app lifespan (existing behavior).
6. Cron: nightly `pg_dump` → `aws s3 cp` (R2 endpoint); 30-day retention.
7. UptimeRobot: HTTP(S) monitor on `https://anchorcore.dev/health`.

## Deploy (every `v*` tag)

GitHub Actions — planned, no workflow yet (extend `release.yml` pattern): build Docker image → push GHCR →
SSH to VPS: `docker compose pull && docker compose up -d`.
Rollback: `docker compose up -d <previous-tag>`.

## Security posture

- No open inbound ports (tunnel egress-only); SSH key-auth only.
- TLS at edge + app: enforce HTTPS, secure cookies.
- Secrets: `.env` on VPS; never in logs (`RedactingFormatter` already in place).
- Tenant isolation: per-workspace row scoping (generalize `projects`/`source_ids`
  pattern); per-workspace R2 prefix/bucket; scoped MCP tokens with audit
  (`system_events`).
- B30 label gating: hosted ingests `public`/`internal` only; `sensitive`/`pii`
  stay local.
- GDPR: export + delete workspace endpoints before first EU customer.

## Cost (fixed)

| Item | ~€/mo |
|---|---|
| Hetzner CX22 | 4–5 |
| anchorcore.dev | $1 |
| Cloudflare / R2 / UptimeRobot | 0 |
| Stripe | 0 + ~3%/payment |
| Postgres (on VPS) | included |
| **Fixed total** | **~€10–15/mo** + ~$2–3 per active workspace in model tokens |

## Scale-out path (only when demand justifies)

1. Postgres → managed (Neon/Supabase); no app change (env-driven DB).
2. App replicas behind Cloudflare load balancing; `JobManager` single-writer
   caveat → queue if needed.
3. R2 already off-box; no storage migration.

## Explicitly excluded until a documented demand signal

Kubernetes, multi-region, autoscaling, managed DBs, service mesh, edge compute.
