# Pilot retro 2026-10-09 — building B54 with an AI agent

Pilot question: how cheap and reliable is building a real feature with an AI
agent? Subject: B54 (dummy-proof model setup) — backend readiness probe + pull
endpoints, wizard/banner UI, 22 tests, docs. One session, start to finish.

## What worked

- **Repo conventions steered without hand-holding.** AGENTS.md (contract-first
  workflow, CCN ≤ 15, error/secret patterns) did the most work of any input.
  The agent followed existing patterns (OnceLock globals, spawn_blocking DB,
  TanStack polling, theme tokens) instead of inventing new ones.
- **Two structured questions replaced a spec.** "Guided vs auto vs loud-only"
  and "blocking vs skip+banner" were asked up front with a recommendation;
  answers pinned the design. No wrong-shape implementation to throw away.
- **Gates caught real errors.** eslint found a JSX syntax error, tsc + vite
  type-checked the build, vitest ran 17/17 (10 new). The implement → gate →
  fix loop worked exactly as intended — for the frontend.
- **Maintained tests came with the feature**, not as an afterthought
  (backend unit tests for matching/validation/NDJSON parsing, frontend tests
  for banner logic and pull roll-ups).
- **Honest verification reporting.** No-linker and no-browser limits were
  stated plainly with what substitutes (rustfmt parse, review, CI gating)
  instead of claimed green.

## What didn't

- **No Rust toolchain on the box — the big gap.** No linker means no
  `cargo check/test/build` and no live backend run. Backend correctness rests
  on pattern-mirroring + review + CI. An agent that can't compile can't fully
  verify compiled-language work; this is the top fix for the next pilot.
- **CRLF friction.** The edit tooling matches exact bytes; repo files are
  CRLF, so multi-line edits needed single-line anchors or throwaway Python
  scripts. Worked, but slow and ugly. (Consider `.gitattributes`
  normalization — owner call, touches every file.)
- **No end-to-end proof.** The actual pull flow never ran against a real
  Ollama; the wizard never rendered against the new backend. Unit tests +
  build are good but not the same as watching the feature work.
- **Context cost scales with feature size.** A cross-stack feature needed
  ~15 file reads before writing. Fine once; smaller scoped tasks would be
  much cheaper per unit of output.

## Conditions for success (preliminary)

1. Conventions doc (AGENTS.md equivalent) exists and is current — highest
   leverage input.
2. Runnable gates exist (lint/test/build) — agent output is trustworthy
   exactly where gates run, and suspect where they don't.
3. Product decisions get asked, not guessed — keep the structured-questions
   habit for anything that changes user-visible shape.
4. Human reviews the final diff and owns the merge — the agent proposed,
   verified mechanically, and disclosed limits; judgment stayed human.

## Suggested next pilots

- Get a linker on the box (or a dev container with the full toolchain) and
  re-run this retro's "what didn't" — measure how much the gap closes.
- Try a deliberately vague task next, to find where the questions-up-front
  habit breaks down.
- Try a pure-bugfix task (e.g. B53) to compare fix-vs-feature economics.
