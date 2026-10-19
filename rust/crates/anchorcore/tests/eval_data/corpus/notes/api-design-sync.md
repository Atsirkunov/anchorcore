# API design sync — 2026-10-02

Attendees: Amara, Luis, Priya. Topic: rate limits for the public api.

## Notes

The api serves third parties now, so the design sync must decide rate
limits before launch. We decide the following limits: one thousand
requests per minute per key, with burst headroom for interactive use.
Rate limits return status 429 with a retry hint. The sync agreed that
every new api route ships with limits from day one.

## Decision

The design sync will decide rollout order next week: gateway limits
first, then per-key limits. Amara owns the api rate limits dashboard.
