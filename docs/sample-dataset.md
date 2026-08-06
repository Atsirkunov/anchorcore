# AnchorCore Sample Dataset (B21)

> NOTE: this file lives in `docs/` on purpose — the folder connector ingests
> everything under `sample/`, so an explanatory README there would be
> classified as company content. Point the source at `sample/` only.

A mini-company corpus for testing AnchorCore end-to-end without real data.
Fictional payments company ("the payments team"): decisions, meetings, specs,
and people — designed to exercise every surface of the product.

## Connect it

In the UI → Sources tab → **Add source**:

- Connector: **Local folder**
- Path: the absolute path to this `sample/` directory (e.g.
  `C:\Users\you\anchorcore\sample`)
- Name: anything, e.g. "Sample company"

Then click **Sync now** (or wait for the watcher). Reclassify after changing
anything.

## What each file exercises

| File | Exercises |
|---|---|
| `sprint12.md` | Original sample: decisions + actions + status; **duplicate partner** for the merge flow |
| `people.md` | Person/owner entities; feeds the future who_knows query |
| `decisions/d1-security-transfers-scope.md` | decision entity with reasoning; **near-duplicate of the sprint12 decision** → review/merge proposal |
| `decisions/d2-billing-provider-approval.md` | decision + approval + constraints + cross-reference to runbook |
| `decisions/d3-mobile-wallet-postponed.md` | decision with revisit trigger (follow-up: "and when do we revisit it?") |
| `decisions/d4-payments-infra-migration.md` | done decision + open item |
| `meetings/m1-sprint12-retro.md` | retro: action items with owners; **disputed-worthy claim** (98% vs 87% coverage); **low-confidence note** (queue provider open question) |
| `meetings/m2-q3-planning.md` | planning: multiple actions + risks + cross-references |
| `specs/security-transfers-spec.md` | **follow-up demo anchor**: numbered movements (initiation → settlement); "how does a transfer work?" → "show the movements for it" |
| `specs/billing-migration-runbook.md` | process doc with numbered steps (another follow-up candidate) |

## Demo scripts (use the Ask tab)

**1. The 60-second wow**
- "What was decided about security transfers, and why?"
- Expect: cited decision with reasoning (excludes incoming transfers).

**2. Follow-ups**
- "How does an outgoing security transfer work?"
- "show the movements for it" — follow-up resolves via history rewriting.
- "and what happens if delivery fails?" — rolls back (step 6 + rollback note).

**3. Review tab states**
- Low confidence: the queue-provider note in the retro should surface in
  Low confidence.
- Duplicates: `sprint12.md` and `decisions/d1` record the *same* decision
  ("Security Transfers MVP excludes incoming transfers"). Whether the merge
  proposal fires depends on the classifier's summary wording — the 3B model
  paraphrases, so it may fall short of the 0.92 similarity threshold. This is
  a known classifier-quality gap (B23); the duplicate flow itself is
  demonstrable with the rulebook's many real duplicate pairs.
- Disputed: the 98%→87% coverage contradiction in `m1` is dispute-worthy —
  dispute it, then confirm answers stop citing it (B3 pending; status flag
  works today).

**4. People & ownership**
- "Who owns the billing migration?" → Sarah, cited from decisions + retro.

## Keeping it honest

- Files are deliberately small and self-referential; they are fixtures for
  testing, not real company data.
- Add your own realistic files to the same folders as you grow the demo.
- The duplicate pair and the coverage contradiction are intentional — don't
  "fix" them, they're the test content.
