# Decision: Security Transfers MVP excludes incoming transfers

**Decision:** The Security Transfers MVP will NOT include incoming transfers.

**Reasoning:** Incoming transfers add material reconciliation complexity —
the receiving side must match external settlement messages, handle partial
deliveries, and reconcile against the counterparty's records. The MVP team
chose to focus on outgoing transfers only, where we control both legs and can
ship a verifiable flow in the current quarter.

**Context:** This is the same decision recorded in the sprint 12 planning
notes. Incoming transfers remain on the roadmap for a follow-up release once
reconciliation tooling lands.

**Status:** Confirmed. Source: PRD v3, section 4.2. Owner: Aisha.

**Follow-ups:**
- Outgoing transfer movement steps are specified in the Security Transfers
  spec (see specs/security-transfers-spec.md).
- Reconciliation tooling for incoming transfers is tracked separately; not
  part of MVP scope.
