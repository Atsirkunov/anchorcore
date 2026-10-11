# Decision: Eval gate thresholds for launch

**Decision:** A build may ship only if the eval suite clears: citation accuracy
>= 0.92, unsupported-claim rate <= 0.03, refusal rate <= 0.02, and p95 latency
<= 2.5s on the reasoning slice.

**Reasoning:** The old gate was "no regressions vs last week", which let two
bad builds through because the baseline itself had drifted. Absolute thresholds
plus a frozen baseline set are what Tomas recommended after the July drift
review. Refusal rate is included deliberately: an assistant that answers
everything is not safer, it is less honest.

**Constraints:**
- The suite must run in CI in under 10 minutes, so the reasoning slice is
  sampled rather than exhaustive.
- The sensitive slice never runs against a cloud provider; it is scored
  locally, and a missing local run fails the gate rather than skipping it.
- Threshold changes require Tomas and Yuki together; no silent re-baselining.

**Status:** Approved. Owner: Tomas. Applies from the September launch.

**See also:** specs/eval-pipeline-runbook.md, decisions/d4-eval-set-contamination.md.
