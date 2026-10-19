# RFP Section 8.2 — Reversal notifications

Each Direct Debit published under this RFP carries a reversal notification schedule.
The beneficiary receives the reversal notification alongside the entitlement record.

## Reason table

| Reason | Meaning |
|--------|---------|
| 01 | Duplicate Direct Debit entry; issue a reversal |
| 02 | Wrong beneficiary identifier; issue a reversal with the corrected identifier |
| 03 | Entitlement paid twice; the second payment needs a full reversal |

A reversal notification is delivered within one business day of the Direct Debit record being amended.
