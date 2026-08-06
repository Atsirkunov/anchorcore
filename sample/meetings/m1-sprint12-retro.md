# Sprint 12 Retro — payments team

Attendees: Sarah, Mike, Priya, Dev, Elena, Aisha.

## What went well

- The new infrastructure migration landed with zero downtime — Dev and Sarah
  ran a textbook dry-run first.
- Billing provider security review completed; findings were minor and are
  resolved.

## What didn't

- The mobile wallet vendor kept changing their auth API; integration spikes
  wasted about a week. Postponed to Q3 by decision.
- Test coverage: we reported 98% before the migration, but Priya noticed the
  number was computed on the old host set. After re-running on the new hosts,
  coverage is actually around 87% — the migration script skipped some legacy
  test modules. Priya will re-baseline and publish the corrected number. Some
  people on the team still quote the old 98% figure in standups.

## Action items

- Priya: re-baseline test coverage on new hosts and publish corrected number.
  Owner: Priya. Due: end of this week.
- Mike: close the two remaining alerting gaps from the migration. Owner: Mike.
- Elena: draft the empty-state screens for the billing admin UI. Owner: Elena.
  Due: next sprint planning.
- Sarah: confirm the queue provider decision — someone suggested we discuss
  moving off the current one, but nobody objected in the meeting, so it was
  left as an open question rather than a decision. Revisit next retro.

## Notes

- The "cover the coverage" numbers discrepancy is worth keeping an eye on —
  there's an open question about whether the corrected 87% still meets the
  release gate.
