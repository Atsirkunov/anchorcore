# Spec: Instant Payments — outgoing payment steps

Scope: MVP = outgoing payments only (see decisions/d1). This spec defines the
steps that make up an outgoing instant payment.

## Payment lifecycle

An outgoing payment is composed of the following steps, in order:

1. **Initiation** — the account owner submits a payment request with the
   payee account and amount. Status: pending validation.
2. **Validation** — the system checks eligibility (payee IBAN is valid,
   account has available balance, limits respected). Rejections are returned
   to the requester with a reason. Status: validated or rejected.
3. **Funds hold** — the amount is held in the ledger: the available balance
   is debited and a pending outgoing balance is credited. No money leaves
   the account yet.
4. **Submission** — the held payment is submitted to the payment scheme for
   the payee's bank. This is the only step that touches the external network.
5. **Payment confirmation** — the payee's bank confirms receipt; the pending
   balance is closed and the payment is marked completed.
6. **Audit record** — every step writes an immutable audit row (who, when,
   what, payment id, amount).

## Key attributes

- Identifiers: IBAN + internal payment id.
- Amounts: always whole cents for MVP (no split payments).
- Limits: per-account daily outgoing cap, enforced at validation step 2.
- Rollback: a failed submission releases the hold (reverse of steps 3-4).

## Notes

- Incoming payments would mirror these steps with the hold and submission
  roles swapped; deliberately out of MVP scope.
- Reconciliation tooling (Dev's spike) will later attach external statements
  to step 4.
