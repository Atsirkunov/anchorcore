# Spec: Model router v2

Approved in decisions/d1. Owner: Noor. Shadow mode first, then cohorts.

## Tiers

1. **small-local** — short factual lookups, retrieval-grounded answers, any
   tenant whose policy is local-only. Default tier.
2. **frontier-api** — multi-step reasoning, long-context synthesis, tenants that
   have explicitly allowed cloud providers.

## Request flow

1. **Policy first** — resolve the tenant's egress policy server-side. Client
   hints are ignored (see INC-114).
2. **Classify** — score the request for reasoning depth and context length.
3. **Pick a tier** — the higher tier only if the score clears the threshold and
   the policy allows it.
4. **Log the decision** — tier, score, policy, latency and cost estimate, so an
   answer can be explained months later.
5. **Fall back** — on frontier timeout, answer from the local tier and mark the
   answer as degraded rather than retrying silently.

## Escalation

- Any request that crosses a cloud boundary outside an approved policy pages
  Sam and Noor immediately.
- Tier thresholds are owned by Noor; changing them requires a shadow run of at
  least one sprint.

## Out of scope

- Per-request model selection beyond the two tiers.
- On-prem deployments (no owner yet).
