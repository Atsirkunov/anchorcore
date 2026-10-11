# Runbook: Eval pipeline

Owner: Tomas. Thresholds come from decisions/d2; the holdout set is frozen per
decisions/d4.

## Steps

1. **Run the fast suite** — sampled reasoning slice plus the full lookup slice.
   Budget: 8 minutes. Fails the build on any threshold miss.
2. **Run the sensitive slice locally** — never against a cloud provider. A
   missing local run fails the gate; it is not skippable.
3. **Compare against the frozen baseline** — the baseline is a versioned set,
   not last week's number.
4. **Record the run** — score, sample size, model versions, router tier mix.
   The launch decision quotes this record, not a spreadsheet.
5. **Publish the corrected number** — if a run invalidates a number people are
   still quoting (see the 0.94 vs 0.89 case), the corrected figure goes into the
   release note, not just the dashboard.

## Escalation

- Threshold miss on citation accuracy blocks the launch; Tomas escalates to
  Yuki with the failing cases attached.
- Any eval case found in the prompt library blocks the run until it is removed.

## Out of scope

- Human review of individual answers (that is the review queue, not the gate).
- Cost reporting; see Ravi's unit-economics sheet.
