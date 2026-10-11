# Sprint 14 planning — Relay platform team

Attendees: Noor, Tomas, Yuki, Sam, Lena, Ravi.

## Where we think we stand

Citation accuracy is 0.94 on the reasoning slice, so the launch gate is not a
concern this sprint. (See d4 — this is the number people were still quoting
before the holdout rebuild.)

## What we committed to

- Router v2 shadow mode behind a flag (Noor), with decision logging included
  from the first build.
- Eval gate thresholds implemented in CI, sampled reasoning slice (Tomas).
- Usage-tier pricing page copy and the credit counter in the account view
  (Yuki + Lena).

## What didn't make the sprint

- Multi-region serving: Sam's estimate doubled once egress controls were
  included. Revisit after the launch.
- On-prem packaging: no owner yet. Yuki to find one or drop it.

## Action items

- Noor: wire router decision logging and publish the flag. Owner: Noor. Due:
  mid-sprint.
- Tomas: rebuild the reasoning holdout set per decision d4; publish the
  corrected accuracy number. Owner: Tomas. Due: end of sprint.
- Sam: add an egress preflight check to the deploy pipeline (postmortem action).
  Owner: Sam. Due: next week.
- Lena: draft the citation format for the lookup tier so short answers still
  show their source. Owner: Lena.
- Yuki: confirm with Ravi that Starter's loss ceiling is unchanged before the
  pricing page ships. Owner: Yuki.

## Open questions (not decisions)

- Should the eval suite run against the customer's own provider config, or only
  ours? Nobody owned it; deferred.
- Someone suggested moving the GPU pool to a cheaper region — no objection in
  the room, but no decision either. Revisit next planning.
