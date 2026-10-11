# Decision: Rebuild the reasoning holdout set

**Decision:** The reasoning holdout set is retired and rebuilt from scratch;
the previous set is kept read-only as `holdout-v1` for historical comparison.

**Reasoning:** Tomas found that 41 of the 600 holdout questions were quoted
verbatim in the prompt-library docs that Lena maintains — the model had been
tuned against examples that were also in the eval set, so the reported numbers
were optimistic. Re-scored on a clean set, citation accuracy on the reasoning
slice was 0.89, not the 0.94 we had been quoting. The launch gate (d2) is 0.92,
so the September launch is currently blocked on this.

**Constraints:**
- New cases must be written by someone who has not written the prompt library
  for that topic (Tomas + one rotation reviewer).
- Prompt-library docs must stop containing eval questions; a CI check rejects
  exact overlaps.
- The rebuilt set is frozen once: 600 cases, versioned, never tuned against.

**Status:** Approved. Owner: Tomas. Rebuild due end of sprint 14.

**See also:** decisions/d2-eval-gate-thresholds.md.
