# Payments service moves to new infrastructure

The payments service is now running on the new infrastructure.

**Reasoning:** The old hosts hit scaling limits during peak settlement and the
deploy pipeline was a manual chore. Dev proposed the migration, Sarah approved
it after a dry-run showed zero-downtime deploys. The move was completed during
the last maintenance window.

**Trade-offs:**
- Deploy time dropped from ~45 minutes to ~5 minutes.
- Two transient alerting gaps occurred during the move; both closed.
- The queue provider stayed the same to reduce risk (see retro note about the
  alternative queue provider).

**Status:** Done. Source: sprint 12 status update. Owner: Sarah.

**Open item:** test coverage was reported at 98% before the move; Priya is
re-running the full suite on the new hosts and will confirm the number.
