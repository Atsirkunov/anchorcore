# AnchorCore — v2/v3 Business & Platform Scoping (draft)

> Goal: turn the validated local-first v1 into a product people can find,
> try, and pay for. Status: **scoping only — decisions needed, nothing built.**
> Everything here builds on the v1 memory core (see [product-plan.md](./product-plan.md)).

## Decisions log (2026-08-10)

Locked decisions from the publishing-strategy review; each maps to a backlog item
(B-numbers in [product-plan.md](./product-plan.md)):

| # | Decision | Backlog |
|---|---|---|
| D1 | Domain: **`anchorcore.dev`** registered (~$12/yr, Cloudflare Registrar). `anchorcore.ai`/`.com`/`.app` are taken but parked — no squatter premium | B33 |
| D2 | Website: static Astro/Vite on Cloudflare Pages (free); landing + docs + changelog; "free during validation" until hosted exists | B33 |
| D3 | Pricing: **sub-only, one dial** — local free forever; sub includes everything (no separate fixed license). Individual €19/mo, Teams €25/user/mo | B35 |
| D4 | Hosted = low-risk, quota-bounded features only (hosted sync, MCP endpoint, webhooks, share links, review); cheap-tier metered bundled models; per-workspace caps | B35 |
| D5 | License: **BSL 1.1** + Ed25519-signed keys, verified offline (air-gapped capable); community carve-out for non-commercial + small orgs | B36 |
| D6 | Enterprise: **sell the platform, don't host it** — Docker + SSO/RBAC/audit, flat annual per deployment + support; enterprise telemetry = SLA feature | B37 |
| D7 | Telemetry: **opt-in, off by default, counts-only** (never content), pseudonymous reset-able ID, transparency screen; public signals (release downloads, stars) complement | B34 |
| D8 | Object storage: **Cloudflare R2** (S3-compatible, free 10 GB + 1M A / 10M B ops/mo, **$0 egress** → free share-link serving) | B35 |
| D9 | Infra topology: **single Hetzner VPS (~€4–5/mo) running Docker Compose (FastAPI + Postgres)**; ingress via **Cloudflare Tunnel** (no open ports); SSL/DNS on the free Cloudflare plan; Stripe for billing; UptimeRobot for uptime pings. One box until real demand — no K8s/multi-region/autoscaling. Full plain-terms plan below (§3). | B35 |

---

## 1. The funnel we're designing

```
Find us (website) → Try it (free tier) → Keep it (local, free) 
→ Need more (hosted sync, sharing, bundled models) → Pay (teams/enterprise)
```

v1 today = the middle two boxes. This doc scopes the rest.

## 2. Website

**Goal:** a landing page that makes the *60-second wow* self-explanatory —
"connect sources → ask → cited answer" — plus docs and a download.

| Page | Content | Notes |
|---|---|---|
| Home | hero + 60s demo (video or interactive), the "AI memory" positioning, 3-step how-it-works, download CTA | Message: *"Preserve company knowledge and make every AI model smarter."* |
| Download | Windows exe + macOS app (both exist — B20/B24), checksums, changelog link | direct artifact links from GitHub Releases (B25) |
| Docs | README + docs/ rendered | could be a static site built from the repo (VitePress/Docusaurus/Mintlify) |
| Pricing | free tier + paid tiers (see §6) | only once hosted exists; until then "free during validation" |
| Changelog | release notes per version | see §5 |
| Blog | founder-led thought leadership (GTM per product-plan §12) | later |

**Stack options (decide later):**
- Static: VitePress or Mintlify on the repo docs — cheap, versioned with code
- Landing: plain static site (Astro/Vite) or a no-code builder first; custom later
- Domain: reserve `anchorcore.ai` (or similar) early regardless

## 3. Hosting (the hosted v2 story)

**Principle: same code, env-driven.** The local app already has the full API
(connectors, settings service, jobs). Hosted = run it on a server with
Postgres + a real object store instead of SQLite + keychain.

| Decision | v1 (local) | v2 (hosted) | Notes |
|---|---|---|---|
| DB | SQLite | PostgreSQL | `DB` swap point already documented |
| Secrets | OS keychain | server-side secret manager | same `SecretStore` interface |
| Vector store | sqlite-vec | pgvector / Qdrant | `VectorStore` swap point |
| Model calls | user's keys / Ollama | bundled keys (margin) or BYO | |
| Connectors | folder + Jira (local) | Jira/Drive/Linear OAuth (server-side) | folder watch is local-only; hosted uses API connectors |
| Files | local folder | object storage (S3-compatible) | Drive connector (B28) fits naturally here |
| Sync | local watcher/poll | server-side scheduled jobs | reuse `JobManager` |

**Ops shape (recommended, pragmatic):** single FastAPI app + Postgres + object
storage on a small VPS (Hetzner/Fly/Render) — one server per region; scale
later. GitHub Actions deploys on tag (same release flow as B25).

**Locked stack (2026-08-10, D8–D9):**

| Piece | Choice | Why |
|---|---|---|
| Front door (DNS/SSL/CDN) | Cloudflare free plan on `anchorcore.dev` | free TLS + DDoS absorption |
| Ingress | **Cloudflare Tunnel** (`cloudflared`) | no open ports on the VPS — nothing to firewall; the tunnel dials out |
| The computer | **Hetzner CX22** (2 vCPU / 4 GB / 40 GB), ~€4–5/mo | cheapest reliable; Docker Compose runs both services |
| App + DB | Docker Compose: `anchorcore` (FastAPI) + `postgres` | one command to start/update; same code, env-driven (DB swap point) |
| Files | Cloudflare R2 (D8) | free 10 GB, $0 egress → share links cost nothing to serve |
| Billing | Stripe Checkout (hosted pages) + webhooks | no PCI burden; the app turns features on when a payment webhook arrives |
| Uptime | UptimeRobot free plan | emails you if the site dies; you don't stare at dashboards |
| Backups | nightly `pg_dump` → R2 | if the computer dies, restore on a new one in ~30 min |
| Deploy | GitHub Actions builds a Docker image on `v*` tag → GHCR → VPS pulls | same tag flow as B25; `docker compose up -d` updates the app |

Deliberately **not** in scope until real demand: Kubernetes, multi-region,
autoscaling, managed DBs. One box, ~€10–15/mo fixed + model tokens
(~$2–3/active workspace). When the box gets small, the app/db/storage split is
already clean: Postgres moves to a managed service (Neon/Supabase), R2 stays.

**Auth:** self-serve signup first (email+password or Google), SSO deferred to
Enterprise (§7).

## 4. File sharing

**What users want:** share a *memory workspace* — "here's our team's knowledge,
ask it questions" — not raw file transfer.

| Capability | v1.5 | v2 | Notes |
|---|---|---|---|
| Read-only share link | ✅ | ✅ | a project (B15) with a token link; the MCP HTTP endpoint (B14.2) is the same mechanism |
| Invite collaborators (read-only) | — | ✅ | per-user tokens, audit via system_events |
| Collaborative review | — | ✅ | teams verify/dispute/merge together |
| Write access (agents) | — | ✅ | MCP `ingest` (B14.3) |
| Permissions (owner/editor/viewer) | — | ✅ | v2 Teams tier trigger |

**Label-scoped sharing (B30):** sharing isn't all-or-nothing. Sources carry a
label (`public|internal|sensitive|pii`); share links, MCP tools, and hosted
workspaces only expose `public` content by default, and `sensitive`/`pii`
content never leaves the local machine (local models only). This makes
"share my team memory" safe *and* is the governance story enterprise buyers
will ask about first.

**The hook:** sharing a workspace is the natural "try the paid tier" moment —
local stays free, shared = hosted + paid.

## 5. Release management & changelogs

Current state (B25): tag `v*` → CI builds Windows + macOS, attaches to GitHub
Releases. Needed next:

- **Changelog file in the repo** (`CHANGELOG.md`) — manual, Keep a Changelog
  format; entries reference backlog items (B-numbers). Release workflow step
  in `docs/releasing.md` already says "changelog if we keep one" — make it
  mandatory.
- **SemVer** — `v0.x.0` until first paid pilot, then `v1.0.0`.
- **Website changelog page** — rendered from CHANGELOG.md.
- **Auto-release notes** — generate from commits between tags (later; manual
  curated changelog is better quality at this stage).
- **Update checks** — the app checks the latest version on launch (opt-in),
  "update available" banner. Deferred; needs a stable artifact URL first.

## 6. Free tier & pricing (aligns with product-plan §10–11)

**Principle: local = free forever (privacy moat); hosted = paid.** Free tier
exists to get people past the download → demo → value gap, not to subsidize
heavy hosted use.

| | Free (local) | Hosted Individual | Hosted Teams | Enterprise |
|---|---|---|---|---|
| Price | €0 | €15–50/mo | €20–50/user/mo | €20k–100k+/yr |
| Core memory + Q&A | ✅ | ✅ | ✅ | ✅ |
| Hosted sync ("AI already knows") | — | ✅ | ✅ | ✅ |
| Bundled model access | — | ✅ (metered) | ✅ | ✅ |
| Team workspace + sharing | — | 1 workspace | unlimited | unlimited |
| Contradictions/verification | — | — | ✅ | ✅ |
| Agentic levels 2–3 | — | — | ✅ | ✅ |
| SSO, audit logs, on-prem | — | — | — | ✅ |

**Free tier decisions to make:**
- Does hosted have a free tier at all (e.g. 1 workspace, 50 MB, 3 sources)?
  Recommendation: yes, small — it's the demo without running anything.
- Local app: always free, no feature-gating (users self-host = word of mouth
  + future enterprise on-prem).
- Metering: storage (GB), sources, model tokens — decide when hosted is real.

## 7. Enterprise (v3 stretch)

- SSO (SAML/OIDC), audit logs, data residency, on-prem/self-hosted enterprise
  support contract.
- Agentic levels 3–4 (draft → prepare-action-with-approval → autonomous) as
  the enterprise productivity premium.
- Compliance: contradiction detection + verification workflows (the
  audit-grade provenance story).

## 8. Suggested sequencing (after v1 validation)

| Step | What | Depends on |
|---|---|---|
| 1 | Changelog discipline + SemVer | now (process) |
| 2 | Website landing + download page | B20/B24 artifacts exist |
| 3 | Docs site | repo docs |
| 4 | Read-only share links (project tokens) | B15 projects, B14.2 MCP HTTP |
| 5 | Hosted pilot (1 server, signup, free tier) | Postgres + auth work |
| 6 | Hosted pricing + Stripe billing | pilot feedback |
| 7 | Teams (invites, permissions) | hosted |
| 8 | Enterprise (SSO, audit, on-prem) | teams + contradictions |

## 9. Open questions

- [x] ~~Domain + name check~~ — **resolved 2026-08-10: `anchorcore.dev`** (B33)
- [ ] Hosting provider (Hetzner/Fly/Render/self-managed)?
- [ ] Hosted free tier size (storage/sources/workspaces)?
- [ ] Signup: email+password vs Google OAuth first?
- [x] ~~Bundled model access — which provider(s), how metered?~~ — **partially resolved 2026-08-10**: cheap-tier metered allowance (~2–3M tokens), expensive models BYO (B35); provider choice TBD
- [x] ~~File sharing: workspace sharing only, or file download too?~~ — **resolved 2026-08-10**: workspace/share-link sharing only (B35)
- [x] ~~Update-checker opt-in from day one or later?~~ — **resolved 2026-08-10**: later, as part of the on-prem managed-update story (B37)
- [x] ~~Changelog: manual curated vs generated?~~ — **resolved 2026-08-10**: manual, Keep a Changelog format (B33)

---

*Companion docs: [product-plan.md](./product-plan.md), [releasing.md](./releasing.md), [architecture.md](./architecture.md), [packaging.md](./packaging.md)*
