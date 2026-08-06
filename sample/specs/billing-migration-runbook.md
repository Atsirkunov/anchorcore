# Runbook: Billing provider migration

Approved in decisions/d2. Owner: Sarah. This runbook is the step-by-step
process for the parallel run.

## Steps

1. **Provision sandbox** — create the new provider tenant and import current
   plan catalog. Verify idempotent webhook behavior.
2. **Feature flag** — add per-tenant billing provider flag; default stays
   legacy.
3. **Parallel run** — for the first cohort of tenants, mirror invoices to the
   new provider (write-only; no customer impact). Compare generated invoice
   amounts for drift.
4. **Cutover cohort** — switch read/write for the first cohort; monitor
   invoice creation and webhook delivery for 48 hours.
5. **Full cutover** — enable the flag for all tenants.
6. **Rollback check** — the flag can flip any tenant back to legacy within
   five minutes; runbook assumes rollback works until the legacy provider is
   decommissioned.

## Escalation

- Daily reconciliation mismatch review during parallel run; Sarah escalates to
  Dev if mismatches exceed 0.5%.
- Billing emails must keep the same templates (see decision constraints).

## Out of scope

- Migration of historical invoices (read-only archive stays on legacy).
