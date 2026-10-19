# R33.6 phase 1 — entity-kind audit: prose taxonomy collapse

> Status: phase-2 GO (owner 2026-10-14: Q1(a) broaden-then-implement, Q2 GO as scoped). §§1–8 are the phase-1 record (kept intact); §9 is the broadened-marker re-audit.
> Date: 2026-10-13. Worktree HEAD `8ecf24d`, binary `anchorcore v1.0.26` (debug).
> Method: §5. Raw entity dump: §6 (every entity, both corpora).

## 1. Headline finding (read this first)

On **both** corpora the current 8-type pipeline extracts **zero decisions, zero
actions, zero `document` entities — 48/48 entities are `note`** — because the
8-way LLM type-detect collapses **all 20 files to `documentation`** (note-only),
including files that literally start with `**Decision:**` and meeting retros
with `Attendees:` + `Action items` sections.

So the taxonomy problem on these corpora is **not** false-positive decisions —
it is **total false negatives**: real decisions arrive as `note` or as nothing
at all. The R33.1 bucket collapse (`record` = meeting/decision_log may emit
decision/action; `reference` = everything else, notes only; deterministic
markers, no LLM detect) fixes the miss side, not the FP side. Two read-only
Ollama probes with the production `meeting` prompt prove recoverability (§4).

## 2. Before/after counts (both corpora)

Before = measured from live syncs (LLM path, no fallbacks fired — §5).
After = projection under deterministic bucket assignment
(markers → `record`, everything else → `reference`; marker set = today's
`detect_rules` markers, §3). Record files keep their kinds and regain
decision/action emission; reference files stay notes-only.

### sample/ (source 1: 10 items, 23 entities)

| kind     | before | after (projected) | note |
|----------|--------|-------------------|------|
| decision | 0      | 0 + recovered¹    | ~10 ground-truth decision-units sit in `record` files (§3); 1 in `reference` (d3 — would-hide, §4) |
| action   | 0      | 0 + recovered¹    | ~10 ground-truth actions, all in `record` files |
| document | 0      | 0                 | no file's ground truth needs `document` as distinct from `note` |
| note     | 23     | 23 (+/− re-extract²) | all survive; record-file notes keep kind, reference-file notes keep kind |

### sample-ai/ (source 2: 10 items, 25 entities)

| kind     | before | after (projected) | note |
|----------|--------|-------------------|------|
| decision | 0      | 0 + recovered¹    | 5 ground-truth decisions, all in `record` files; none would hide |
| action   | 0      | 0 + recovered¹    | ~8 ground-truth actions, all in `record` files |
| document | 0      | 0                 | same as above |
| note     | 25     | 25 (+/− re-extract²) | all survive |

¹ Exact recovered decision/action counts need the phase-2 re-extract (new
prompts, new windows). Spot-probed: `m1-sprint12-retro.md` 1 note → 1 decision
+ 3 actions + 1 note; `d3-mobile-wallet-postponed.md` 0 entities → 1 decision
(§4). ² Re-running extraction re-segments notes; the count will move — the
guarantee is *no kind is deleted by the bucket line*, not an identical count.

## 3. Per-file table (the eyeball table)

`detect` = today's LLM doc_type (all `documentation`).
`bucket` = deterministic assignment (meeting markers: attendees/agenda/meeting
notes/minutes of/retro; decision_log markers: decision log/decision:/adr-/
status: accepted — the `detect_rules` set, applied to the 12k head).
`truth` = ground truth from reading the file (D = real decision, A = real
action, – = neither).

### sample/

| file | detect | entities today | truth | bucket (marker hit) | after | verdict |
|------|--------|----------------|-------|---------------------|-------|---------|
| decisions/d1-instant-payments-scope.md | documentation | 1 note | 1D | record (`decision:`) | decision recoverable | RECOVERED (projected) |
| decisions/d2-billing-provider-approval.md | documentation | 0 | 1D | record (`decision:`) | decision recoverable | RECOVERED (projected) |
| decisions/d3-mobile-wallet-postponed.md | documentation | 0 | 1D | **reference (no marker)** | stays notes-or-nothing | **WOULD HIDE — flag §4** |
| decisions/d4-payments-infra-migration.md | documentation | 1 note | 1D | record (`retro` — incidental, "see retro note") | decision recoverable | RECOVERED but FRAGILE §4 |
| meetings/m1-sprint12-retro.md | documentation | 1 note | 1D + 4A (Sarah item explicitly *not* a decision) | record (`attendees`, `retro`) | probed: 1D+3A+1N | RECOVERED (probed) |
| meetings/m2-q3-planning.md | documentation | 9 notes | 3D + 4A | record (`attendees`, `agenda`) | decisions/actions recoverable | RECOVERED (projected) |
| people.md | documentation | 4 notes | – | reference | notes unchanged | CORRECT, unchanged |
| specs/billing-migration-runbook.md | documentation | 6 notes | – (procedural steps) | reference | notes unchanged | CORRECT, unchanged |
| specs/instant-payments-spec.md | documentation | 1 note | – | reference | notes unchanged | CORRECT, unchanged |
| sprint12.md | documentation | 0 | 3D + 2A | record (`decision log` — genuine `## Decision log`) | recoverable | RECOVERED (projected) |

### sample-ai/

| file | detect | entities today | truth | bucket (marker hit) | after | verdict |
|------|--------|----------------|-------|---------------------|-------|---------|
| decisions/d1-model-router-v2.md | documentation | 1 note | 1D | record (`decision:`) | decision recoverable | RECOVERED (projected) |
| decisions/d2-eval-gate-thresholds.md | documentation | 1 note | 1D | record (`decision:`) | decision recoverable | RECOVERED (projected) |
| decisions/d3-pricing-usage-tiers.md | documentation | 1 note | 1D | record (`decision:`) | decision recoverable | RECOVERED (projected) |
| decisions/d4-eval-set-contamination.md | documentation | 1 note | 1D | record (`decision:`) | decision recoverable | RECOVERED (projected) |
| meetings/m1-sprint14-planning.md | documentation | 1 note | 0 explicit D + 5A (2 explicit non-decisions) | record (`attendees`; also incidental `decision log` ⊂ "decision logging") | actions recoverable | RECOVERED (projected) |
| meetings/m2-incident-postmortem.md | documentation | 1 note | 1D ("Decided no for now") + 3A | record (`attendees`) | recoverable | RECOVERED (projected) |
| people.md | documentation | 4 notes | – | reference | notes unchanged | CORRECT, unchanged |
| specs/eval-pipeline-runbook.md | documentation | 8 notes | – | reference | notes unchanged | CORRECT, unchanged |
| specs/model-router-spec.md | documentation | 6 notes | – | reference | notes unchanged | CORRECT, unchanged |
| sprint14.md | documentation | 1 note | 0 explicit D (goal + commitments) | reference | stays a note | CORRECT-ish — eyeball³ |

³ `sprint14.md` commitments ("Committed: router v2 shadow mode…") are
decision-*adjacent*. Under `reference` they stay notes — findable, just not
kinded. Owner call whether commitments should be record markers (see Q1).

## 4. No-lost-decision check + concrete examples

**Check result: 1 would-hide, 1 fragile, 0 lost elsewhere.**
"Lost" = a ground-truth decision/action whose file lands in `reference`
(notes-only) under deterministic markers. Nothing is byte-deleted either way —
chunks still index every file; the question is only whether the entity keeps a
decision/action kind.

- **WOULD HIDE: `sample/decisions/d3-mobile-wallet-postponed.md`.** A real,
  confirmed decision ("Aisha and Elena agreed…", "the decision was made to
  defer", "**Status:** Confirmed") with no `decision:` marker — title is
  `# Mobile wallet…` (no `Decision:` prefix) and the body jumps from prose to
  `**Reasoning:**`. Markers-as-shipped put it in `reference`. Proven
  recoverable: the production `meeting` prompt extracts `[decision] Mobile
  wallet integration postponed to Q3 (conf 1.0)` from this exact text.
- **FRAGILE: `sample/decisions/d4-payments-infra-migration.md`.** Real decision
  content ("Sarah approved it after a dry-run…") but also no `decision:`
  marker — it lands in `record` only via incidental `retro` ("see retro note
  about the…"). Right bucket, wrong reason; a reword drops it to `reference`.
- **Marker-substring noise (non-blocking):** `sample-ai/m1` hits `decision log`
  via "decision logging" (genuine `attendees` carries it anyway). Consider
  word-boundary matching in phase 2.
- **Correctly excluded:** both `people.md` (4+4 notes, responsibilities),
  both runbooks (6+8 notes, procedural steps), both specs (1+6 notes, facts),
  `sprint14.md` (1 note, goal/commitments) — `reference` fits all of them.

**Decisions-arriving-as-`note` (the inverted FP table — what the buckets fix):**

| file | today (all `note`) | ground truth in that text |
|------|--------------------|---------------------------|
| m2-q3-planning.md | "Instant Payments MVP scope confirmed…", "Billing provider migration parallel-run starts Monday.", "Wallet integration stays on hold." | 3 decisions |
| m2-q3-planning.md | "Dev: build the reconciliation tooling spike…", "Priya: define the release coverage gate…", "Mike: draft the payment fraud controls…", "Aisha: book the partner re-engagement call…" | 4 actions with owners |
| m1-sprint12-retro.md | "The new infrastructure migration landed with zero downtime." (only entity; 4 action items + 1 decision missed) | 1D + 4A, see probe below |
| d1-instant-payments-scope.md | "The Instant Payments MVP will NOT include incoming payments." | the file's `**Decision:**` verbatim, kinded `note` |
| d4-payments-infra-migration.md | "The payments service moved to new infrastructure…" | "Sarah approved it" flattened to a fact |
| ai d1–d4 (each 1 note) | "Approved router v2…", "Builds ship only if…", "Pricing moves…", "The reasoning holdout set is retired…" | 4× the file's `**Decision:**`, all kinded `note` |
| ai m1-sprint14-planning.md | "Citation accuracy is 0.94 on the reasoning slice." (only entity — the *stale* number; 5 action items missed) | 5A missed; the extracted note is the known-bad metric |
| (zero-entity files) | d2, d3, sprint12.md (sample) → 0 entities despite 5D + 2A between them | the detect-then-note-only path can also yield nothing |

**Recoverability probes** (read-only, production `meeting` prompt,
`llama3.2:3b`, temperature 0.1 — no code changed):
- `m1-sprint12-retro.md`: 1 note → `[decision] Postponing mobile wallet vendor
  integration to Q3` + 3 actions (re-baseline coverage / alerting gaps /
  empty-state screens) + 1 note. The Sarah open-question item is correctly
  *not* extracted as a decision (confirms the prompt respects "rather than a
  decision").
- `d3-mobile-wallet-postponed.md`: 0 entities → `[decision] Mobile wallet
  integration postponed to Q3 (conf 1.0)`.

## 5. Method (reproducible)

1. Worktree @ `8ecf24d` (clean); `rust/target` + `frontend/dist` copied from
   the main checkout (`cp -R`); `cargo check -p anchorcore --offline` green;
   debug binary built (`cargo build -p anchorcore --offline`).
2. Server: `./rust/target/debug/anchorcore --port 8261 --data-dir
   /tmp/r336-audit` with `ANCHOR_OPEN_BROWSER=0 ANCHOR_SECRETS_NO_KEYRING=1
   ANCHOR_TRAY=0` (port probed free first; server stopped after, temp dir
   outside the repo).
3. `POST /sources` (folder, label `internal`) + `POST /sources/{id}/sync` per
   corpus; polled `GET /sources/jobs/{id}` to `done`. Job results:
   sample/ `10/10, 23 entities, ~38s`; sample-ai/ `10/10, 25 entities, ~43s`.
   Ollama local (`llama3.2:3b` + `nomic-embed-text`), so classify + detect ran
   the LLM path; server log shows **0** `document-type detection failed` and
   **0** `document-type gate` lines — no rules fallback involved.
4. Counts + summaries read from `/tmp/r336-audit/anchorcore.db`
   (`ingested_items.doc_type` × `entities.kind`, `entities.summary`).
5. Detect collapse double-check: the exact production `TYPE_DETECT_PROMPT` +
   payload shape replayed against `localhost:11434` for 6 files (2 decisions,
   2 meetings, 1 runbook, 1 postmortem) → all 6 returned `documentation`.
   The 20/20 `documentation` verdict is genuine model behavior, not a harness
   artifact. (Why the 3B model latches onto `documentation` is unknown —
  eldest-middle-option bias is a guess, not a finding.)
6. Bucket column computed with the `detect_rules` marker set (quoted in §3);
   ground-truth column by reading all 20 files.

## 6. Raw dump (all 48 entities — the complete evidence)

Format: `source | file | kind | summary` (`1` = sample/, `2` = sample-ai/;
all parents `doc_type=documentation`).

```text
1 | decisions/d1-instant-payments-scope.md | note | The Instant Payments MVP will NOT include incoming payments.
1 | decisions/d4-payments-infra-migration.md | note | The payments service moved to new infrastructure during the last maintenance window.
1 | meetings/m1-sprint12-retro.md | note | The new infrastructure migration landed with zero downtime.
1 | meetings/m2-q3-planning.md | note | Instant Payments MVP scope confirmed for outgoing payments only.
1 | meetings/m2-q3-planning.md | note | Billing provider migration parallel-run starts Monday.
1 | meetings/m2-q3-planning.md | note | Wallet integration stays on hold.
1 | meetings/m2-q3-planning.md | note | Dev: build the reconciliation tooling spike for incoming payments.
1 | meetings/m2-q3-planning.md | note | Priya: define the release coverage gate at the corrected baseline.
1 | meetings/m2-q3-planning.md | note | Mike: draft the payment fraud controls before the spec review.
1 | meetings/m2-q3-planning.md | note | Aisha: book the partner re-engagement call for the wallet vendor.
1 | meetings/m2-q3-planning.md | note | Billing parallel run may surface reconciliation mismatches.
1 | meetings/m2-q3-planning.md | note | If Mike's payment fraud controls slip, the spec review date moves.
1 | people.md | note | Mike must sign off before merge for security-sensitive changes.
1 | people.md | note | Sarah is the final approver for anything touching payout or money movement.
1 | people.md | note | Priya owns the definition of done for release (coverage gate, smoke suite).
1 | people.md | note | Aisha is the point of contact for external partners.
1 | specs/billing-migration-runbook.md | note | Provision sandbox: create new provider tenant and import current plan catalog.
1 | specs/billing-migration-runbook.md | note | Feature flag: add per-tenant billing provider flag; default stays legacy.
1 | specs/billing-migration-runbook.md | note | Parallel run: mirror invoices to new provider for first cohort of tenants.
1 | specs/billing-migration-runbook.md | note | Cutover cohort: switch read/write for first cohort; monitor invoice creation and webhook delivery for 48 hours.
1 | specs/billing-migration-runbook.md | note | Full cutover: enable flag for all tenants.
1 | specs/billing-migration-runbook.md | note | Rollback check: flag can flip any tenant back to legacy within five minutes.
1 | specs/instant-payments-spec.md | note | An outgoing payment is composed of six steps: Initiation, Validation, Funds hold, Submission, Payment confirmation, and Audit record.
2 | decisions/d1-model-router-v2.md | note | Approved router v2 for tiered routing with small-local for short lookups and frontier-api for multi-step reasoning.
2 | decisions/d2-eval-gate-thresholds.md | note | Builds ship only if eval suite clears citation accuracy >= 0.92, unsupported-claim rate <= 0.03, refusal rate <= 0.02, and p95 latency <= 2.5s.
2 | decisions/d3-pricing-usage-tiers.md | note | Pricing moves from per-seat to three usage tiers with monthly credit allowances.
2 | decisions/d4-eval-set-contamination.md | note | The reasoning holdout set is retired and rebuilt from scratch.
2 | meetings/m1-sprint14-planning.md | note | Citation accuracy is 0.94 on the reasoning slice.
2 | meetings/m2-incident-postmortem.md | note | A router v2 shadow build ignored the customer egress policy for 19 minutes and sent 214 requests to the frontier API.
2 | people.md | note | Noor and Sam need to sign off on changes to prompt sending locations.
2 | people.md | note | Yuki owns the launch criteria; Tomas owns whether a build meets them.
2 | people.md | note | Ravi must approve any change to price, credits or included usage.
2 | people.md | note | Tomas is the point of contact for the external eval vendor.
2 | specs/eval-pipeline-runbook.md | note | Thresholds come from decisions/d2.
2 | specs/eval-pipeline-runbook.md | note | The holdout set is frozen per decisions/d4.
2 | specs/eval-pipeline-runbook.md | note | Budget: 8 minutes for the fast suite.
2 | specs/eval-pipeline-runbook.md | note | A missing local run fails the gate; it is not skippable.
2 | specs/eval-pipeline-runbook.md | note | The baseline is a versioned set, not last week's number.
2 | specs/eval-pipeline-runbook.md | note | Record the run: score, sample size, model versions, router tier mix.
2 | specs/eval-pipeline-runbook.md | note | The launch decision quotes this record, not a spreadsheet.
2 | specs/eval-pipeline-runbook.md | note | If a run invalidates a number people are still quoting, the corrected figure goes into the release note.
2 | specs/model-router-spec.md | note | Model router v2 has two tiers: small-local and frontier-api.
2 | specs/model-router-spec.md | note | The default tier is small-local for short factual lookups and retrieval-grounded answers.
2 | specs/model-router-spec.md | note | Tenants with explicitly allowed cloud providers are classified under frontier-api.
2 | specs/model-router-spec.md | note | The request flow involves policy resolution, classification, tier selection, logging, and fallback.
2 | specs/model-router-spec.md | note | Tier thresholds are owned by Noor and require a shadow run of at least one sprint to change.
2 | specs/model-router-spec.md | note | Model selection is out of scope for per-request beyond the two tiers.
2 | sprint14.md | note | Sprint goal is to land the router v2 dry run and freeze the eval gate for the September launch.
```

Zero-entity files (3): `sample/decisions/d2-billing-provider-approval.md`,
`sample/decisions/d3-mobile-wallet-postponed.md`, `sample/sprint12.md`.

## 7. Stop/go questions for the owner

**Q1 — bucket-line call (d3 + d4).** Markers-as-shipped hide one real decision
(sample d3, no `decision:` marker) and catch another only by accident (sample
d4, incidental `retro`). Options: (a) broaden the `record` markers before
phase 2 (candidates: `\bagreed\b|\bdecided\b|\bapproved\b`,
`**Status:** (Confirmed|Accepted|Approved)`, `# Decision` title, `revisit
trigger`) and re-run this audit (cheap — same script, temp dir); (b) accept
markers-as-shipped — d3's content stays findable as chunks/notes, kinds are
metadata; the per-source override (D6 escape hatch) covers real-world
stragglers. Which?

**Q2 — GO on phase 2?** The audit reframes the win: the collapse fixes *total
false negatives* (0 decisions extracted today), not FPs. Phase 2 = deterministic
bucket assignment + one strict prompt per bucket (`record` may emit
decision/action, `reference` notes-only; `code` already skips) + delete the
8-way LLM detect + per-type prompt matrix + `DISTILL_DOC_TYPES` folded into the
bucket definition. GO as scoped, or re-scope first (e.g. keep a `document`
kind, or split `record` into meeting vs decision_log prompts)?

## 8. Phase-2 test plan (not implemented — convention note)

Repo convention is maintained Rust tests, `cargo test -p anchorcore` green.
Phase 2 should add: (a) bucket-assignment unit tests over fixture texts —
each marker fires, `decision logging` ≠ `decision log` (word boundaries),
d3/d4-shaped texts land per the Q1 call; (b) a pipeline test that a
`record` window may yield decision/action while a `reference` window yields
notes only (mock classifier or rules-level assertion, must pass without
Ollama); (c) a re-sync test that prose entities/conflicts/review state survive
the bucket switch (no purge — unlike R33.2 code); (d) an audit-replay test
pinning the §2 counts on the two corpora (or a fixed fixture pair) so the
bucket line can't silently drift. No code written in phase 1.

## 9. Re-audit with broadened markers (phase-2 GO record)

> Date: 2026-10-14. Branch `rust/R33.6-phase2` off `5eb14d8`, binary
> `anchorcore v1.0.26` (debug, pre-phase-2 code). Same §5 method: port 8261
> (probed free), `--data-dir /tmp/r336p2` (outside the repo), local Ollama
> (`llama3.2:3b` + `nomic-embed-text`); server stopped after. Job results:
> sample/ `10/10, 23 entities, ~42s`; sample-ai/ `10/10, 27 entities`.
> Server log shows **0** `document-type detection failed` and **0**
> `document-type gate` lines — LLM path, no rules fallback.

### Broadened record-marker set (owner Q1(a), all word-boundaried, 12k head)

Old `detect_rules` set, word-boundaried: `\battendees\b`, `\bagenda\b`,
`\bmeeting notes\b`, `\bminutes of\b`, `\bretro\b`, `\bdecision log\b`,
`\bdecision:`, `\badr-`, `\bstatus:\s*accepted\b`. Plus: `\bagreed\b`,
`\bdecided\b`, `\bapproved\b`, `\*\*status:\*\*\s*(confirmed|accepted|approved)`
(markdown `**Status:**` verdict line), `(?m)^#{1,6}\s+decision\b` (`# Decision`
titles, any ATX level), `\brevisit triggers?\b` (plural-tolerant — the
wild occurrence is singular d3, but `triggers` is the natural plural form).

### Live re-sync: phase-1 headline reconfirmed

Still **20/20 `documentation`**, still **50/50 entities `note`** (23 + 27),
zero decisions/actions/`document`. Zero-entity files now: sample d2, d3
(sprint12 yields 6 notes this run vs 0 in phase 1 — LLM re-segmentation
variance, all still `note`). The taxonomy problem remains total false
negatives, not FPs.

### Per-file buckets under the broadened set

`truth` unchanged from §3 (D = real decision, A = real action, – = neither).

### sample/ — 8 record / 2 reference

| file | bucket (marker hits) | truth | verdict |
|------|----------------------|-------|---------|
| decisions/d1-instant-payments-scope.md | record (`decision:`, `**Status:**`, `# Decision`) | 1D | RECOVERED (projected) |
| decisions/d2-billing-provider-approval.md | record (`decision:`, approved, `**Status:**`, `# Decision`) | 1D | RECOVERED (projected) |
| decisions/d3-mobile-wallet-postponed.md | record (**agreed**, **Status: Confirmed**, **revisit trigger**) | 1D | **RECOVERED — was WOULD HIDE** (3 markers, §4 flag cleared) |
| decisions/d4-payments-infra-migration.md | record (**approved** genuine + incidental `retro`) | 1D | **RECOVERED robust — was FRAGILE** ("Sarah approved it" carries it) |
| meetings/m1-sprint12-retro.md | record (`attendees`, `retro`) | 1D + 4A | RECOVERED (probed §4) |
| meetings/m2-q3-planning.md | record (`attendees`, `agenda`) | 3D + 4A | RECOVERED (projected) |
| people.md | reference (no marker) | – | CORRECT, unchanged |
| specs/billing-migration-runbook.md | record (**approved** — "Approved in decisions/d2") | – (procedural steps) | **WEAK FLIP — watch** (provenance pointer, not a decision; strict record prompt must emit nothing here) |
| specs/instant-payments-spec.md | reference (no marker) | – | CORRECT, unchanged |
| sprint12.md | record (`decision log`, decided, `# Decision` via `## Decision log`) | 3D + 2A | RECOVERED (projected) |

### sample-ai/ — 7 record / 3 reference

| file | bucket (marker hits) | truth | verdict |
|------|----------------------|-------|---------|
| decisions/d1-model-router-v2.md | record (`decision:`, approved, `**Status:**`, `# Decision`) | 1D | RECOVERED (projected) |
| decisions/d2-eval-gate-thresholds.md | record (`decision:`, approved via Status line, `**Status:**`, `# Decision`) | 1D | RECOVERED (projected) |
| decisions/d3-pricing-usage-tiers.md | record (`decision:`, approved via Status line, `**Status:**`, `# Decision`) | 1D | RECOVERED (projected) |
| decisions/d4-eval-set-contamination.md | record (`decision:`, approved via Status line, `**Status:**`, `# Decision`) | 1D | RECOVERED (projected) |
| meetings/m1-sprint14-planning.md | record (`attendees` only — "decision logging" noise **gone** via word boundary) | 5A | RECOVERED (projected) |
| meetings/m2-incident-postmortem.md | record (`attendees`, decided + approved) | 1D + 3A | RECOVERED (projected) |
| people.md | reference (no marker) | – | CORRECT, unchanged |
| specs/eval-pipeline-runbook.md | reference (no marker) | – | CORRECT, unchanged |
| specs/model-router-spec.md | record (**approved** — "Approved in decisions/d1" + adjectival "an approved policy") | – | **WEAK FLIP — watch** (same provenance-pointer shape as the sample runbook) |
| sprint14.md | reference (no marker) | 0 explicit D (goal + commitments) | CORRECT-ish — commitments stay notes (no commitment markers per Q1(a) as specified) |

### Re-audit findings

- **d3/d4 both cleared.** d3 lands in `record` on three independent broadened
  markers; d4 on genuine `approved` (the incidental `retro` is now backup, not
  the carrier). **No-lost-decision check: 0 would-hide, 0 fragile** (was 1 + 1).
- **2 weak flips, both the same shape:** a spec/runbook opening with an
  "Approved in decisions/dX" provenance pointer (plus one adjectival "an
  approved policy"). The bucket is a *permissive* gate (`record` = MAY emit
  decision/action) — the strict per-bucket prompt is the real filter, and a
  pointer with no recorded choice+reasoning must yield no decision. Both files
  are flagged for the phase-2 live verification (re-sync must show notes-only
  or nothing from them, no fabricated decisions).
- **Word boundaries behave:** "decision logging" no longer hits `decision log`;
  "approver"/"must approve" (both people.md) don't hit `approved`; "Revisit
  next retro" doesn't hit `revisit trigger`.
- **`# Decision` titles are redundant on these corpora** (every such file also
  hits `decision:`) but principled — kept as specified.
- **Unchanged reference set otherwise:** both people.md, both remaining specs,
  eval runbook, sprint14 — all correctly `reference`.
