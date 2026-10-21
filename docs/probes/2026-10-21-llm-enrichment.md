# Probe 2026-10-21: LLM enrichment vs rules on unstructured docs (R45.11 input)

One-shot value test for post-processing LLM enrichment: does a chat model
find real decisions/actions in prose the record parser skips, at a precision
worth shipping? Corpus + runner are session scratch (`/tmp/probe-corpus`,
`/tmp/probe_enrich.py`, raw outputs in `/tmp/probe-results.json`); the docs
below were written before either side ran, and the LLM side got one prompt,
no tuning.

**Headline:** the model (llama3.2:3b, temp 0) finds real buried items the
rules miss, but at ~40–55% precision — every content-free, tentative, or
deferred doc produces confident junk. That is the exact failure mode that
motivated cutting the LLM from ingest. Do not build R45.11 on this evidence
with this model.

## 1. Setup

- Corpus: 10 docs, 713 words. 8 unstructured prose (buried real content in
  `sync-prose`, `incident-review`; tentative/deferred in `planning-thread`,
  `design-chat`; nothing actionable in `status-update`, `vendor-email`,
  `retrospective`; one commitment in `standup-notes`) + 2 structured
  controls (`adr-control`, `actions-control`).
- Rules side: release binary, folder sync through the public API, read back
  via `GET /decisions` + `GET /actions` (+ `state=all`, `GET /entities`).
- LLM side: direct Ollama `/api/chat`, `llama3.2:3b`, JSON-only prompt
  ("only include things the document clearly states as decided or
  assigned"), whole doc per call, lenient `{…}` parse.
- Cost: rules sync 2.0 s for all 10 docs; LLM 13.2 s (~1 s/doc) — ~3.7 h
  per 10k docs on this machine.

## 2. Results

| Doc | Rules | LLM | Verdict |
|---|---|---|---|
| sync-prose | 0 | Postgres decision ✓, Luis benchmark ✓ +owner | LLM win, fully real |
| incident-review | 0 | 2 real follow-ups, wrong bucket (decisions), owners missed | LLM partial win |
| planning-thread | 0 | vendor trial as DECIDED (doc: explicitly deferred) | over-claim |
| design-chat | 0 | direction "decided" (doc: "still open") + 2 borderline mockup actions | fabrication + borderline |
| standup-notes | 0 | "No blockers" decision (fabricated) + 1 borderline pairing action | mixed |
| status-update | 0 | decision + action from a content-free status | fabrication |
| vendor-email | 0 | vendor's price hike as our decision | fabrication |
| retrospective | 0 | doc title regurgitated as a decision (doc: "observation, not a plan") | fabrication |
| adr-control | decision + action + owner, exact | same + 1 empty junk action | tie (LLM +junk) |
| actions-control | **0 (see §3)** | 2 checkbox items, wrong bucket, 1/2 owners | LLM partial win |

Tally on the 8 unstructured docs: ~4 solid + 3 borderline real rows vs ~7
junk rows. Owners missed on most real finds. Buckets (decision vs action)
confused in 3 of 10 docs.

## 3. Rules gap found: `RECORD_PATTERNS` has no action-items marker

`actions-control.md` (`## Action items` + checkboxes, nothing else) yields
zero entities — not a parser failure, a bucket failure:
`classify_rules_typed` returns early unless `assign_bucket` says `record`,
and none of the 12 `RECORD_PATTERNS` match an action-items section or a
checkbox. A doc whose only record marker is its action list is invisible.
Fix is deterministic (add `\baction items\b` + checkbox markers to the
patterns); precision risk is low because the record parser itself stays
strict. Filed as R45.14, not done in this probe.

(Note the two gates disagreed in the good direction twice: `retrospective`
and `incident-review` bucketed as record via "retro"/"agreed", the parser
ran, and correctly produced nothing.)

## 4. Implications for R45.11

- The recall win is real but the precision (~50% at 3b) is not shippable:
  junk would land as unverified rows in Decisions/Actions — re-polluting
  the views R45.8 just cleaned — and the review queue that could have
  absorbed it was deleted in R45.13 on owner feedback.
- If enrichment is revisited: re-probe first with a bigger local model
  and/or a tuned prompt (few-shot, tentative-vs-decided instruction).
  Ship only if precision clears ~85% on this corpus; keep `unverified`
  always (R45.8 rule stands).
- Suggested order: (1) the §3 pattern fix, (2) optional re-probe with a
  stronger model, (3) R45.11 design only on a passing re-probe.
