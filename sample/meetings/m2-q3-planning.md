# Q3 Planning — instant payments & billing

Attendees: Aisha, Sarah, Mike, Priya, Dev.

## Agenda outcomes

1. Instant Payments MVP scope confirmed: outgoing payments only (see
   decisions/d1). Incoming payments deferred until reconciliation tooling.
2. Billing provider migration parallel-run starts Monday. Sarah owns it.
3. Wallet integration stays on hold (see decisions/d3).

## New action items

- Dev: build the reconciliation tooling spike for incoming payments so we can
  size the follow-up release. Owner: Dev. Due: Q3 week 3.
- Priya: define the release coverage gate at the corrected baseline. Owner:
  Priya. Due: Q3 week 1.
- Mike: draft the payment fraud controls (auth, rate limits, audit
  log) before the spec review. Owner: Mike. Due: before spec review.
- Aisha: book the partner re-engagement call for the wallet vendor once their
  contract freezes. Owner: Aisha.

## Risks

- Billing parallel run may surface reconciliation mismatches; Sarah will
  monitor daily and escalate via the rollback flag if needed.
- If Mike's payment fraud controls slip, the spec review date moves.

## Cross-references

- Instant payment steps: specs/instant-payments-spec.md
- Billing migration steps: specs/billing-migration-runbook.md
