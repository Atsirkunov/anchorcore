# Try the AI startup

> The second demo corpus lives in the repo's `sample-ai/` folder — clone the repo
> (or grab the folder from GitHub) and point a source at it, exactly like
> `sample/`. Release zips ship the app only.

A fictional AI product company ("Relay"): model routing, evals, pricing
decisions and an incident postmortem — built to exercise what the payments
corpus doesn't: policy-gated routing, threshold launch gates, and a metric that
had to be corrected.

## Connect it

In the app → Sources tab → **Add source**:

- Connector: **Local folder**
- Path: the absolute path to the `sample-ai/` directory — click **Browse…** to navigate, or paste it (e.g.
  `C:\Users\you\anchorcore\sample-ai`)
- Name: anything, e.g. "Relay"

Then click **Sync now**.

## What's inside

| File | What's in it |
|---|---|
| `sprint14.md` | Sprint goals, what was committed and carried over |
| `people.md` | People and who owns what |
| `decisions/d1-model-router-v2.md` | A tiered-routing decision with constraints |
| `decisions/d2-eval-gate-thresholds.md` | Launch gates as absolute thresholds |
| `decisions/d3-pricing-usage-tiers.md` | A pricing decision with margin reasoning |
| `decisions/d4-eval-set-contamination.md` | A corrected metric that blocks a launch |
| `meetings/m1-sprint14-planning.md` | Planning notes: actions, an open question, a stale number |
| `meetings/m2-incident-postmortem.md` | A postmortem: what happened, why, what changed |
| `specs/model-router-spec.md` | The router spec with a numbered request flow |
| `specs/eval-pipeline-runbook.md` | The eval runbook with thresholds and escalation |

Two things in here are deliberate test content, not mistakes: the planning notes
still quote the old eval number (0.94) that `d4` corrects to 0.89, and the
postmortem's open question about publishing an incident note was never closed.
They exist so you can try the Review tab's dispute flow and watch an answer
change its source.

## Things to try (in the Ask tab)

**1. The 60-second wow**
- "What did we decide about model routing, and why?"
- Expect: the tiered-routing decision with its reasoning, cited — 62% less spend
  on the lookup slice, policy constraints included.

**2. Policy-gated answers**
- "Which tier can a tenant with a local-only policy reach?"
- Answered from the router spec and the postmortem, not from guesswork.

**3. A corrected number**
- "What is our citation accuracy on the reasoning slice?"
- → 0.89 on the rebuilt holdout set, with `d4` explaining why 0.94 was wrong and
  which launch gate it blocks.

**4. Owners and sign-off**
- "Who approves a change to routing policy?" → Noor and Sam.
- "Who signs off pricing changes?" → Ravi, per `people.md` and `d3`.

**5. Postmortem follow-through**
- "What did we change after prompts reached the cloud provider?" → the three
  changes, each with an owner and a due date.
