# Decision: Billing provider migration approved

**Decision:** We approved moving to the new billing provider.

**Reasoning:** The security review by Mike passed with no blocking findings
after the data-retention gap was closed. The new provider gives us usage-based
pricing, idempotent webhooks, and a proper sandbox, which the legacy provider
lacked. Migration cost is estimated at two sprints and is owned by Sarah.

**Constraints:**
- Must not regress existing invoices; a parallel-run period is required.
- Customer-facing billing emails keep the same templates during migration.
- Rollback plan: feature flag toggles back to the legacy provider per tenant.

**Status:** Approved. Owner: Sarah. Due: end of next week per sprint 12
action items.

**See also:** specs/billing-migration-runbook.md for the step-by-step process.
