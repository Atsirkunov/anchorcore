# Gated access spec — deterministic reads (BYOM) + team consumer gating

Status: Part A (Phase 1) shipped on `rust/R14.X`; Part B is backlog. Goal: users
work with their favorite model (Claude, ChatGPT, local) over MCP with
zero-copy-paste friction, while two orthogonal policies hold:
(1) sensitive data never goes to unapproved *providers*, (2) data never goes to
unauthorized *humans*. Enforcement is deterministic — no LLM in the policy loop.

## Part A — Deterministic gated reads ("bring your own model")

### Principle

"Ask the model to return everything as-is" is rejected as a pattern: lossy
(paraphrase, truncation), non-deterministic, token-wasteful, and un-auditable.
The blessed shape is **deterministic bytes out, model prose out of the harness**:

`search` (ranked, scored, cited) → `get_entity` / `get_source` by ID (verbatim)
→ the caller's model composes. AnchorCore's own answer model becomes optional.

### A1. One gate function on every read — but only on the extraction plane

Today `gate_answer_hits` runs only in `ask`; `search` and the entity detail
reads are ungated. Fix: generalize to `gate_reads(caller, hits)` and apply it
where bytes flow toward a model, NOT where the owner manages their own data:

- Gate always: `/qa`, `/qa/search` (no UI consumer — MCP + AskTab inherit it).
- Gate on explicit opt-in: `/entities/{id}`, `/entities/{id}/related`,
  `/entities/{id}/context` enforce only with `?gate=provider`, which the MCP
  sidecar always passes and the UI never does.
- Never gate: list views, review queues, PII review, source metadata. These are
  the owner's management plane — UI-fetched bytes never leave for a provider
  API, so gating them would lock the owner out of their own data. (The owner
  can open the sqlite file directly; gating localhost reads from the owner is
  theater, not security.)

Inputs per hit: source label (`public|internal|sensitive|pii`), chunk/item PII
flag. Inputs per caller: provider-trust (Part A concern) + membership/tier
(Part B concern). Output: `(allowed, blocked[])` + a machine-readable reason
per block. Pure function, no I/O except label lookup — unit-testable with fixtures.

### A2. Refusal contract (machine-readable)

When relevant hits exist but none survive gating, reads return a structured
refusal instead of silent emptiness, so the harness can react (switch provider,
request access, escalate) deterministically:

- `gated_provider` — blocked by provider-trust (fix: trusted/local provider).
- `gated_membership` — caller not in the project (fix: request access).
- `gated_tier` — caller's clearance below the label tier (fix: admin review).
- `empty` — genuinely nothing found (distinct from blocked — never conflate).

`ask` keeps its human-readable refusal text; it must also carry the code.

### A3. Efficiency for context windows

- `search`: compact-by-default (`fields=` selector: `summary,source_ref,score`
  vs full `content`), `k` cap stays, add cursor pagination (`cursor`/`limit`)
  so harnesses can walk result sets completely and deterministically.
- `get_entity` / chunk fetch: verbatim drill-down by stable ID — the "everything
  as-is" path, one entity at a time, exact bytes.
- No bulk-dump endpoint in phase 1; ranked retrieval + drill-down covers Q&A.
  Add list/export only when a sync-everything use case demands it.

### A4. Tool routing (steer the agent, zero protocol churn)

Rewrite the MCP tool descriptions so agents route themselves:

- `search` — default extraction: "Gated, verbatim, cited chunks for the agent
  to compose from. Prefer this; your model writes the answer."
- `ask` — "Policy-gated composed answer. Use when material may be sensitive or
  you want the server to enforce PII/provider policy before you see bytes."

### A5. Audit

Every gated decision logs to `system_events`: tool, actor (Part B), hit/source
IDs, reason code. "Why didn't I see it" must be answerable from logs.

## Part B — Consumer gating for teams (reuse their IAM, don't rebuild it)

### Principle

Companies will not gate chunks manually and will not adopt a parallel user
directory. So: **integrate with their IdP, mirror minimally, enforce locally.**

- Identity via OIDC (Google Workspace, Okta, Entra ID). Local `users` remain
  for personal/small-team use. SCIM provisioning is phase 3, not phase 1.
- Groups mirrored from the IdP into local `teams`, membership cached and
  refreshed on a schedule. The data plane never calls the IdP per request:
  access decisions read local tables only (available, fast, testable).
- Admin maps IdP groups → teams once; after that, joins/leaves flow from the
  IdP. AnchorCore never becomes the system of record for employment.

### B1. Access model: three axes, no per-chunk work

Effective access = **membership ∩ tier clearance**, evaluated per source:

1. **WHERE — project membership.** Projects (already the source-grouping
   primitive) become the access boundary: `project_members(user/team,
   project, role=admin|member)`. A company gates a *group of sources* by
   putting them in a project, once — never chunk by chunk.
2. **WHAT TIER — label clearance.** Labels (`public|internal|sensitive|pii`,
   per source, default `internal`) + automatic PII flags are policy tiers,
   orthogonal to membership. Teams get a clearance ceiling, e.g. Contractors:
   `public+internal`, never `sensitive/pii` — even inside a shared project.
3. **WHO — IdP teams.** Users inherit team memberships from the mirror.

Example: "Salaries" source (label `sensitive`) in project "HR" (members: HR
team). An engineer in "Eng" but not "HR": denied by membership. A contractor
in "HR" with clearance `internal`: denied by tier. Both denials carry distinct
codes (A2), both are audited.

### B2. Defaults (safe, boring, correct)

- Default-deny: no membership → invisible (list-filtered, get-by-ID → 404, not
  403, so membership can't be probed).
- New source: inherits creator's current project, label `internal`.
  Sources in no project: admin-only.
- Personal installs: single implicit owner of everything — zero behavior change,
  migration is a no-op.

### B3. Enforcement points (all of them, or none of it counts)

Retrieval scope (extend existing `project_scope` with user clearance — embeddings
stay shared, filtering happens at query time), graph expansion (already
scope-aware — keep it that way), list endpoints (filter, don't just gate items),
ask/search/entity/source reads, MCP tools. One scope-builder function shared by
all paths; divergence here is how leaks happen.

### B4. Explainability

Access must be answerable: a dry-run/preview ("why can X see Y" / "what does
team T see") backed by the same pure function as enforcement. Companies will
ask this in procurement; support will need it on day two.

### B5. MCP identity

Per-user API tokens: `Authorization` header on HTTP transport, env-provided
token for the stdio sidecar mapped to that user. Service accounts for bots
(labeled, scoped like a member). No token → no data; voluntary flags like
`public_only` become unnecessary once identity is real.

## Interplay (the two gates on one read)

|  | Trusted provider (local/listed) | Untrusted provider (public LLM) |
|---|---|---|
| **Member, cleared** | full bytes, compose anywhere | gated bytes only (sensitive stripped, audited) |
| **Non-member / below tier** | 404 + `gated_membership`/`gated_tier` | 404 (membership checked first — never confirm existence) |

Provider-gating protects against third-party *processing*; membership protects
against wrong-human *viewing*. Both run; membership first.

## Phasing

- **Phase 1 (shipped)** — Part A: provider-trust gate on `/qa`, `/qa/search`
  (always) and `/entities/{id,related,context}` (with `?gate=provider`, which
  the MCP sidecar always passes and the UI never does); `gated_provider`
  refusal codes + `blocked` counts; `search` compact-by-default (`fields=`) +
  `cursor`/`limit`/`next_cursor` pagination; tool-description routing;
  gated-decision audit to `system_events` (migration 13 backfills the
  `created_at` default older chains lack). No schema beyond this — the actor
  column arrives with Part B tokens.
- **Backlog (team hosted)** — everything team-only ships together, not piecemeal:
  Part B core (`project_members`, per-user MCP tokens, scope extension, dry-run
  preview, actor audit), OIDC login + IdP group mirror/sync, SCIM, Slack
  integration and similar team surfaces. Rationale: team access is one coherent
  release (identity + membership + enforcement + audit); shipping half of it
  creates the illusion of security without the substance.
- **Deferred/rejected**: per-chunk ACLs (rejected — project/source granularity
  is the ceiling), per-user encryption (API boundary suffices), public share
  links.

## Open questions

1. Orphan sources (no project): admin-only (proposed) vs visible-to-all-members?
2. Tier names: fixed four labels vs admin-customizable tiers? (Proposal: fixed —
   custom tiers make cross-company reasoning impossible.)
3. Who maps IdP groups → teams — first admin via UI, or config-file seed?
