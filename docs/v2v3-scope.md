# AnchorCore — v2/v3 Business & Platform Scoping

> Goal: turn the validated local-first v1 into a product people can find,
> try, and pay for. Status: **partly executed** — website, Team licensing,
> and the hosting skeleton shipped; hosted service, sharing links, and
> changelog discipline are still open.
> Everything here builds on the v1 memory core (see [product-plan.md](./product-plan.md)).

## Decisions log (2026-08-10, with later revisions noted)

Locked decisions from the publishing-strategy review; each maps to a backlog item
(B-numbers in [product-plan.md](./product-plan.md)):

| # | Decision | Status |
|---|---|---|
| D1 | Domain: **`anchorcore.dev`** (Cloudflare Registrar). `.ai`/`.com`/`.app` taken but parked | Done — site targets `anchorcore.dev` |
| D2 | Website: static single page (plain Vite); landing + guides + download; personal free forever, team one-time license, no prices on site | Shipped — live at anchorcore.dev (Cloudflare Workers, B47) |
| D3 | Pricing: personal local free forever; Team self-hosted = one-time platform license (perpetual + 1yr maintenance, offline ed25519 key — shipped: `LICENSE.md` + `backend/app/license.py` + `scripts/make_license.py`); hosted pricing TBD from pilots — no per-seat | Decided + Team half shipped |
| D4 | Hosted = low-risk, quota-bounded features only (hosted sync, MCP endpoint, webhooks, share links, review); cheap-tier metered bundled models; per-workspace caps | Plan — hosting skeleton + auth shipped, service design open (B49) |
| D5 | License: **custom source-available license** (`LICENSE.md`: personal use free, commercial/Team use paid, ed25519 keys verified offline). *Revised — the original BSL 1.1 decision was dropped; there is currently no small-org carve-out.* | Shipped |
| D6 | Enterprise: **sell the platform, don't host it** — Docker + SSO/RBAC/audit, flat annual per deployment + support; enterprise telemetry = SLA feature | Plan |
| D7 | Telemetry: **opt-in, off by default, counts-only** (never content), pseudonymous reset-able ID, transparency screen; public signals (release downloads, stars) complement | Decided, not implemented — no telemetry exists anywhere yet |
| D8 | Object storage: **Cloudflare R2** (S3-compatible, free 10 GB + 1M A / 10M B ops/mo, **$0 egress** → free share-link serving) | Plan |
| D9 | Infra topology: **single Hetzner VPS (~€4–5/mo) running Docker Compose (app + Postgres)**; ingress via **Cloudflare Tunnel** (no open ports); SSL/DNS on the free Cloudflare plan; Stripe for billing; UptimeRobot for uptime pings. One box until real demand — no K8s/multi-region/autoscaling. Full plain-terms plan below (§3). | Locked, not provisioned |

---

## 1. The funnel we're designing

```
Find us (website) → Try it (free download) → Keep it (local, free)
→ Need more (hosted sync, sharing, bundled models) → Pay (teams/enterprise)
```

v1 covers find → try → keep. This doc scopes the rest.

## 2. Website

**Shipped:** a single-page static site (plain Vite, `website/`) — hero with
Personal/Team/Hosted toggle, 60-second demo, guides, FAQ, pricing (no
prices), roadmap/waitlist, privacy notice. Download buttons link straight at
the latest GitHub Release assets for Windows/macOS.

| Piece | State |
|---|---|
| Home | ✅ hero + demo + how-it-works + download CTA |
| Download | ✅ direct `releases/latest/download` asset links (site + README) |
| Docs | GitHub repo itself (no separate docs site — decided against VitePress-style) |
| Pricing | ✅ tiers without prices (see §6) |
| Changelog | ❌ none yet — no `CHANGELOG.md`, no page (see §5) |
| Blog | Later |

Still open: hosting choice (Cloudflare Pages is an option, not chosen),
`og-card.png` export for strict social crawlers, reserving `anchorcore.ai`.

## 3. Hosting (the hosted v2 story)

**Principle: same code, env-driven.** The local app already has the full API
(connectors, settings service, jobs). Hosted = run it on a server with
Postgres + a real object store instead of SQLite + keychain. Per R10.7 the
hosted image stays Python even though local ships Rust.

| Decision | v1 (local) | v2 (hosted) | Notes |
|---|---|---|---|
| DB | SQLite | PostgreSQL | `DB` swap point already documented |
| Secrets | OS keychain | server-side secret manager | same `SecretStore` interface |
| Vector store | sqlite-vec | pgvector / Qdrant | `VectorStore` swap point |
| Model calls | user's keys / Ollama | bundled keys (margin) or BYO | |
| Connectors | folder, Drive, Jira, Linear, REST | Jira/Drive/Linear OAuth (server-side) | folder watch is local-only; hosted uses API connectors |
| Files | local folder | object storage (S3-compatible) | |
| Sync | local watcher/poll | server-side scheduled jobs | reuse `JobManager` |

**Ops shape (recommended, pragmatic):** one app + Postgres + object storage
on a small VPS (Hetzner per D9) — one server per region; scale later. GitHub
Actions deploys on tag (planned — no workflow yet).
**Skeleton (shipped):** `hosting/` (Dockerfile + `docker-compose.yml` +
`.env.example`) runs the same image locally with `pgvector/pgvector:pg16` —
no provider lock-in, no fork. Same `ANCHOR_DATABASE_URL` switch drives
SQLite→Postgres; see `hosting/README.md`.

**Locked stack (2026-08-10, D8–D9):**

| Piece | Choice | Why |
|---|---|---|
| Front door (DNS/SSL/CDN) | Cloudflare free plan on `anchorcore.dev` | free TLS + DDoS absorption |
| Ingress | **Cloudflare Tunnel** (`cloudflared`) | no open ports on the VPS — nothing to firewall; the tunnel dials out |
| The computer | **Hetzner CX22** (2 vCPU / 4 GB / 40 GB), ~€4–5/mo | cheapest reliable; Docker Compose runs both services |
| App + DB | Docker Compose: `anchorcore` (Python image per R10.7) + `postgres` | one command to start/update; same code, env-driven (DB swap point) |
| Files | Cloudflare R2 (D8) | free 10 GB, $0 egress → share links cost nothing to serve |
| Billing | Stripe Checkout (hosted pages) + webhooks | no PCI burden; the app turns features on when a payment webhook arrives |
| Uptime | UptimeRobot free plan | emails you if the site dies; you don't stare at dashboards |
| Backups | nightly `pg_dump` → R2 | if the computer dies, restore on a new one in ~30 min |
| Deploy | GitHub Actions builds a Docker image on `v*` tag → GHCR → VPS pulls | planned, no workflow yet |

Deliberately **not** in scope until real demand: Kubernetes, multi-region,
autoscaling, managed DBs. One box, ~€10–15/mo fixed + model tokens
(~$2–3/active workspace). When the box gets small, the app/db/storage split is
already clean: Postgres moves to a managed service (Neon/Supabase), R2 stays.

Full operational reference: [hosting.md](./hosting.md) — topology, provisioning,
deploy, security posture, backups, scale-out path.

**Auth:** self-serve signup first (email+password or Google), SSO deferred to
Enterprise (§7). (`/auth` signup/login already exists in the app behind
`ANCHOR_AUTH_SECRET`; Google OAuth is the open part.)

## 4. File sharing

**What users want:** share a *memory workspace* — "here's our team's knowledge,
ask it questions" — not raw file transfer.

| Capability | v1.x | v2 | Notes |
|---|---|---|---|
| Read-only share link | — | planned | a project with a token link; needs the MCP HTTP endpoint (B14.2) |
| Invite collaborators (read-only) | — | planned | per-user tokens, audit via system_events |
| Collaborative review | — | planned | teams verify/dispute/merge together |
| Write access (agents) | — | planned | MCP `ingest` (B14.3) |
| Permissions (owner/editor/viewer) | — | planned | v2 Teams tier trigger |

**Label-scoped sharing (mechanism shipped, links not):** sources carry a
label (`public|internal|sensitive|pii`); the share-safe API (`/qa/public`,
`public_only`) already excludes non-public content, and `sensitive`/`pii`
content never leaves the local machine. The actual share *links* are still to
build on top of this. The governance story is what enterprise buyers will ask
about first.

**The hook:** sharing a workspace is the natural "try the paid tier" moment —
local stays free; team self-hosted = paid platform license; hosted = paid when it exists.

## 5. Release management & changelogs

Current state: tag `v*` → CI builds Windows + macOS (+ Linux) zips, attaches
to GitHub Releases. Versions are already 1.x (the old "stay v0 until first
pilot" plan lapsed). Still missing:

- **Changelog file in the repo** (`CHANGELOG.md`) — manual, Keep a Changelog
  format. Make it part of the release checklist in `docs/releasing.md`.
- **Website changelog page** — rendered from CHANGELOG.md (open).
- **Auto-release notes** — generate from commits between tags (later; manual
  curated changelog is better quality at this stage).
- **Update checks** — the app checks the latest version on launch (opt-in),
  "update available" banner. Deferred; no longer blocked (stable
  `releases/latest/download` URLs now exist).

## 6. Free tier & pricing (aligns with product-plan §10–11)

**Principle: personal local = free forever (privacy moat); team self-hosted =
one-time platform license; hosted = paid when it exists.** No per-seat, no
prices on site — price discovery happens in pilot conversations.

| | Personal (local) | Team self-hosted | Hosted | Enterprise |
|---|---|---|---|---|
| Price | €0 | one-time license, perpetual incl. 1yr support/updates | TBD from pilots | support contract, scoped per deal |
| Core memory + Q&A | ✅ | ✅ | ✅ | ✅ |
| License enforcement | — | offline ed25519 key (`backend/app/license.py`) | — | ✅ |
| Hosted sync ("AI already knows") | — | — | ✅ | ✅ |
| Bundled model access | — | — | ✅ (metered) | ✅ |
| Team workspace + sharing | — | ✅ (your VPC) | ✅ | ✅ |
| Contradictions/verification | — | — | ✅ | ✅ |
| Agentic levels 2–3 | — | — | ✅ | ✅ |
| SSO, audit logs, on-prem | — | — | — | ✅ |

**Remaining decisions:**
- Does hosted have a free tier at all (e.g. 1 workspace, 50 MB, 3 sources)?
  Recommendation: yes, small — it's the demo without running anything.
- Hosted price points — set only after pilot conversations, never on the site first.
- Metering: storage (GB), sources, model tokens — decide when hosted is real.

## 7. Enterprise (v3 stretch)

- SSO (SAML/OIDC), audit logs, data residency, on-prem/self-hosted enterprise
  support contract.
- Agentic levels 3–4 (draft → prepare-action-with-approval → autonomous) as
  the enterprise productivity premium.
- Compliance: contradiction detection + verification workflows (the
  audit-grade provenance story).

## 8. Suggested sequencing (after v1 validation)

| Step | What | State |
|---|---|---|
| 1 | Changelog discipline (`CHANGELOG.md`) | open |
| 2 | Website landing + download page | ✅ shipped |
| 3 | Docs presence | decided: GitHub repo itself, no separate docs site |
| 4 | Read-only share links (project tokens) | open, needs B14.2 MCP HTTP |
| 5 | Hosted pilot (1 server, signup, free tier) | open, skeleton in `hosting/` |
| 6 | Hosted pricing + Stripe billing | open, needs pilot feedback |
| 7 | Teams (invites, permissions) | open, needs hosted |
| 8 | Enterprise (SSO, audit, on-prem) | open, needs teams + contradictions |

## 9. Open questions

- [x] ~~Domain + name check~~ — **resolved 2026-08-10: `anchorcore.dev`**
- [x] ~~Hosting provider~~ — **resolved per D9: Hetzner VPS** (not provisioned yet)
- [ ] Hosted free tier size (storage/sources/workspaces)?
- [ ] Signup: email+password vs Google OAuth first?
- [x] ~~Bundled model access — which provider(s), how metered?~~ — **partially resolved 2026-08-10**: cheap-tier metered allowance (~2–3M tokens), expensive models BYO; provider choice TBD
- [x] ~~File sharing: workspace sharing only, or file download too?~~ — **resolved 2026-08-10**: workspace/share-link sharing only
- [x] ~~Update-checker opt-in from day one or later?~~ — **resolved 2026-08-10**: later, as part of the on-prem managed-update story
- [x] ~~Changelog: manual curated vs generated?~~ — **resolved 2026-08-10**: manual, Keep a Changelog format (not started yet)

---

*Companion docs: [product-plan.md](./product-plan.md), [releasing.md](./releasing.md), [architecture.md](./architecture.md), [packaging.md](./packaging.md)*
