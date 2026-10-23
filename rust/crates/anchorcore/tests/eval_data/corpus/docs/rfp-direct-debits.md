# RFP Section 8.2 — Reversal notifications

Each Direct Debit published under this RFP carries a reversal notification schedule.
The payer receives the reversal notification alongside the mandate record.

## Reason table

| Reason | Meaning |
|--------|---------|
| 01 | Duplicate Direct Debit entry; issue a reversal |
| 02 | Wrong payer identifier; issue a reversal with the corrected identifier |
| 03 | Collection taken twice; the second collection needs a full reversal |

A reversal notification is delivered within one business day of the Direct Debit record being amended.
