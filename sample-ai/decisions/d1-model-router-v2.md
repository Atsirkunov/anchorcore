# Decision: Model router v2 (tiered routing)

**Decision:** We approved router v2, which picks a serving tier per request —
`small-local` for short factual lookups, `frontier-api` for multi-step
reasoning — instead of sending everything to one provider.

**Reasoning:** Roughly 70% of our traffic is short lookups that the local model
answers at a fraction of the cost and latency. Frontier spend grew 3.1x last
quarter while answer quality on the reasoning slice was flat. Noor's shadow run
showed the same answer quality on the lookup slice with 62% less spend.

**Constraints:**
- A request may only reach `frontier-api` when the customer's egress policy
  allows cloud providers; Relay's own default stays local-only.
- Router decisions must be logged with the reason (tier, score, policy) so an
  answer can be explained later.
- Rollback: the tier policy ships behind a flag; shadow mode first, then the
  first cohort, then default on.

**Status:** Approved. Owner: Noor. Dry run starts sprint 14.

**See also:** specs/model-router-spec.md, meetings/m2-incident-postmortem.md.
