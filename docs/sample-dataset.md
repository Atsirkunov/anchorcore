# Try the sample company

> Download [sample.zip](https://anchorcore.dev/sample.zip) from the website and unzip it anywhere, then point your source at the extracted `sample/` folder. (The zip is built from the repo's `sample/` at every site build; release zips ship the app only.)

A fictional payments company ("the payments team"): decisions, meetings, specs,
and people — designed so you can try every part of the product in a minute.

> Prefer a software company as the example? The repo ships a second corpus in
> `sample-ai/` — see the AI startup guide (`docs/sample-ai.md`).

## Connect it

In the app → Sources tab → **Add source**:

- Connector: **Local folder**
- Path: the absolute path to the `sample/` directory — click **Browse…** to navigate, or paste it (e.g.
  `C:\Users\you\anchorcore\sample`)
- Name: anything, e.g. "Sample company"

Then click **Sync now** (new and changed files are picked up automatically
after that).

## What's inside

| File | What's in it |
|---|---|
| `sprint12.md` | Decisions, actions and status from a sprint |
| `people.md` | People and who owns what |
| `decisions/d1-instant-payments-scope.md` | A decision with its reasoning |
| `decisions/d2-billing-provider-approval.md` | A decision with an approval and constraints |
| `decisions/d3-mobile-wallet-postponed.md` | A postponed decision with a revisit date |
| `decisions/d4-payments-infra-migration.md` | A finished decision plus an open item |
| `meetings/m1-sprint12-retro.md` | Retro notes: action items with owners |
| `meetings/m2-q3-planning.md` | Planning notes: actions, risks, cross-references |
| `specs/instant-payments-spec.md` | A spec with numbered steps — made for follow-up questions |
| `specs/billing-migration-runbook.md` | A runbook with numbered steps |

Two things in here are deliberate test content, not mistakes: the same
decision appears twice (in `sprint12.md` and `d1`), and one meeting note
contradicts another. They exist so you can try the Entities tab's duplicate
merging and dispute flow (Needs a look + Possible doubles).

## Things to try (in the Ask tab)

**1. The 60-second wow**
- "What was decided about instant payments, and why?"
- Expect: the decision with its reasoning, cited.

**2. Follow-ups**
- "How does an outgoing instant payment work?"
- "show the steps for it" — the follow-up is understood from context.
- "and what happens if delivery fails?" — answered from the rollback note.

**3. Decisions, actions and review (in the Entities tab)**
- Decisions: the four decision records with their status and owner — read
  from the files' own `Status:` / `Owner:` lines, no model involved.
- Actions: the retro and planning action items with owners; filter to open ones.
- Possible doubles: the twice-recorded decision is there to try merging.
- Disputed: dispute the contradicting claim, and answers stop citing it.
- Everything starts unverified — verifying a record is your call.

**4. People & ownership**
- "Who owns the billing provider migration?" → Sarah, cited from the decisions and notes.
