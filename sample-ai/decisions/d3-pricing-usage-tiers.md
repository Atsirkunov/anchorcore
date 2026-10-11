# Decision: Pricing moves to usage tiers

**Decision:** Relay moves from per-seat pricing to three usage tiers with a
monthly credit allowance (Starter 2M tokens, Team 25M, Scale custom), with
overage billed per 1M tokens.

**Reasoning:** Per-seat pricing punished exactly the behaviour we want —
giving more of the team access. Usage tracks cost, and cost is what we actually
pay for. The Team tier is priced at 35% gross margin at the median observed
consumption, which Ravi signed off on; the Starter tier runs at a loss on
purpose as the try-before-you-buy path.

**Constraints:**
- Existing annual contracts stay on per-seat until renewal; no mid-term
  migrations.
- Credits reset monthly and never roll over, except on the Scale tier.
- Any discount above 20% needs Ravi; anything below Starter's loss ceiling
  needs Yuki and Ravi together.

**Status:** Approved. Owner: Yuki. Pricing page ships with the September launch.

**See also:** meeting actions in meetings/m1-sprint14-planning.md.
