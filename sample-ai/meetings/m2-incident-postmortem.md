# Postmortem — prompts reached the cloud provider (INC-114)

Attendees: Noor, Sam, Tomas, Lena, Yuki.

## What happened

A router v2 shadow build ignored the customer egress policy for 19 minutes and
sent 214 requests to the frontier API from tenants whose policy was local-only.
The preflight check added after this is now mandatory in the deploy pipeline.

## Impact

- 214 requests, 6 tenants. No customer documents contained personal data in the
  window that was affected (verified from the request log), so no notification
  was required. Three customers asked for the log; Sam provided it.
- Cost impact negligible. Trust impact is the real one: two of the six were in
  an active security review.

## Why it happened

The shadow build read the tier from the request body instead of re-deriving it
from the policy service, so a stale client could ask for `frontier-api`
directly. The policy service itself was correct.

## What we changed

- The router now derives the tier server-side and ignores client hints for
  anything policy-relevant (Noor).
- Egress preflight runs before deploy and fails closed (Sam).
- Alert on any request that crosses a cloud boundary outside an approved
  policy, with the tenant and the reason (Sam).

## Action items

- Noor: add the server-side tier derivation test that would have caught this.
  Owner: Noor. Due: done.
- Sam: publish the preflight check documentation. Owner: Sam. Due: this week.
- Tomas: add a policy-compliance case to the eval suite so a build cannot pass
  the gate while violating egress rules. Owner: Tomas. Due: sprint 15.

## Notes

- Open question for Yuki: do we publish a public incident note? Decided no for
  now, because no customer data was exposed.
