# Spec: Instant Payments — outgoing payment movements

Scope: MVP = outgoing payments only (see decisions/d1). This spec defines the
movements that make up an outgoing instant payment.

## Payment lifecycle

An outgoing payment is composed of the following movements, in order:

1. **Initiation** — the account owner submits a transfer request with the
   target account and security identifiers. Status: pending validation.
2. **Validation** — the system checks eligibility (security is eligible,
   account has holdings, limits respected). Rejections are returned to the
   requester with a reason. Status: validated or rejected.
3. **Reservation** — the payments are reserved in the ledger: the source
   position is debited and a pending outgoing position is credited. No
   payout happens yet.
4. **Delivery** — the reserved payments are delivered to the payee
   account. This is the only movement that touches the external ledger.
5. **Remittance advice** — the payee confirms receipt; the
   pending position is closed and the transfer is marked completed.
6. **Audit record** — every movement writes an immutable audit row (who,
   when, what, payment id, amount).

## Key attributes

- Identifiers: IBAN + internal payment id.
- Amounts: always whole cents for MVP (no split payments).
- Limits: per-account daily outgoing cap, enforced at validation step 2.
- Rollback: a failed delivery returns the reservation (reverse of steps 3-4).

## Notes

- Incoming payments would mirror these movements with the reservation and
  delivery roles swapped; deliberately out of MVP scope.
- Reconciliation tooling (Dev's spike) will later attach external statements
  to movement step 4.
