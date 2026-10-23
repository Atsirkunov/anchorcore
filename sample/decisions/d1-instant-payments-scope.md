# Decision: Instant Payments MVP excludes incoming payments

**Decision:** The Instant Payments MVP will NOT include incoming payments.

**Reasoning:** Incoming payments add material reconciliation complexity —
the receiving side must match external bank statements, handle returned
payments, and reconcile against the payer bank's records. The MVP team
chose to focus on outgoing payments only, where we control both legs and can
ship a verifiable flow in the current quarter.

**Context:** This is the same decision recorded in the sprint 12 planning
notes. Incoming payments remain on the roadmap for a follow-up release once
reconciliation tooling lands.

**Status:** Confirmed. Source: PRD v3, section 4.2. Owner: Aisha.

**Follow-ups:**
- Outgoing payment steps are specified in the Instant Payments
  spec (see specs/instant-payments-spec.md).
- Reconciliation tooling for incoming payments is tracked separately; not
  part of MVP scope.
