# Sprint 12 planning notes

We decided to postpone the mobile wallet integration to Q3 because the API
contracts are not stable yet. Approval was given for the new billing provider
after the security review.

Action items:
- Sarah owns the migration to the new billing provider, due end of next week.
- Follow up with the design team about the empty state screens.

Status update: the payments service is now running on the new infrastructure,
and 98% of test coverage is green as of today.

## Decision log

Decision: Security Transfers MVP excludes incoming transfers.
Reason: Reconciliation complexity.
Source: PRD v3.
