# Handover 2026-10-21 (EOD): 1.1.0 shipped + probe + wizard interim

For the next agent / owner. Supersedes the *status* parts of
`docs/handover-2026-10-21-phase45-progress.md` (all of its §3.1 decisions
are answered, §4 release done, worktrees deleted).
`docs/handover-2026-10-21-llm-modes.md` stays the Phase 45 design
reference (§2 findings, §4 target design).

## 1. State of the tree

- `main` at `23bdb56`, **in sync with `origin/main`**. Last tag `v1.1.0`
  (pushed; CI Release published with `AnchorCore-{windows,macos,linux}.zip`
  + `SHA256SUMS`, notes attached from `docs/release-notes/v1.1.0.md`).
- Commits since the morning handover (`30ba4b4`):
  - `8a6a5be` Release 1.1.0 (version, notes, opt-in flip, lint fix, CI deselects)
  - `23bdb56` Wizard mode-first interim + enrichment probe + R45.14/R45.11 backlog
- Gates on the release: `cargo test` 534 + 104 green, strict `cargo check`
  clean, lizard at the 7-warning baseline, frontend build + lint + vitest
  71/71 green, conformance 94 passed / 18 deselected (1 macOS-only env
  failure in the sqlite-extension latency probe — never touches the server),
  release binary smoked end to end (sync → decisions/actions/ask).
  `23bdb56` touches no Rust code (wizard + docs); frontend gates re-ran
  green on it, plus a live wiring check (mode flip → `answer_ready`, then
  `agent_compose` with citations, no warning).
- Leftovers: the 6 `worktree-agent-*` **branches** still exist locally
  (worktrees removed, all merged — delete at will). The R45.3 candle probe
  still lives only in a session scratchpad; rebuild from
  `docs/probes/2026-10-21-embedding-engine.md`.

## 2. Release 1.1.0: what shipped

Agent mode (`llm_mode` + `agent_compose`, MCP and REST), Decisions/Actions
REST + MCP + UI, people-owned glossary replacing "Words to confirm",
ingest with zero chat calls by default, vector provenance + re-embed on
model change, record-only extraction with no auto-verify, vec0-upgrade fix.
Migrations 21–24, all verified on a copy of the live DB (upgrades on the
owner's next run of the new build; live DB untouched).

Release-process notes for next time:

- Conformance needed 12 new deselects (frozen suite asserts deleted
  behavior): 7 × R45.8 (prose fixtures expecting entities — ingest itself
  still passes: items stored, totals/history green) + 5 × R45.9 (deleted
  who-knows/graph legs). Same commented style as the R33.7/R35.1 precedents.
- `skills-lock.json` needed no change: it pins third-party contributor
  skills only; our skills ship embedded in the binary, so the rebuild is
  the bump.
- New-binary checklist item that bit: `RecordsViews.tsx` tripped
  `react-hooks/set-state-in-effect` (2 errors) — fixed with the repo's
  `.then()` convention. The release checklist's `npm run lint` gate earned
  its place.

## 3. Owner decisions taken today (all §3.1 closed, in `decisions.md`)

| # | Decision |
|---|---|
| 7.2.1 | Built-in embedder ships **nomic v1.5 stored f16** (~274 MB, pinned SHA-256s in the probe doc) |
| 7.2.2 | **Built-in everywhere**; suggest Ollama only when no Metal + large first index + reachable |
| 7.2.3 | R45.10: **demote to `unverified` + delete** rule rows from non-record items, backup first |
| new | Acronym expansion is **opt-in** (`acronym_expansion=true`; shipped that way in 1.1.0) |
| new | Glossary re-entry: **nothing to do** (owner DB had no approved LLM rows) |
| release | **1.1.0** (minor: new APIs + agent mode + 4 migrations), then tag + push |

Unblocks: R45.4 → R45.6 → R45.7 and R45.10. R45.3's CI-only legs (MinGW,
x86 mkl, Ollama CPU-only baseline) still need a CI run before the engine
decision entry is final.

## 4. Probe: LLM enrichment value (`docs/probes/2026-10-21-llm-enrichment.md`)

One-shot test of the R45.11 idea — llama3.2:3b over 10 docs (8
unstructured prose + 2 controls), rules via the release binary:

- **Verdict: ~40–55% precision — don't build R45.11 on this.** Real buried
  finds (Postgres decision, Luis benchmark, 2 incident follow-ups) vs ~7
  confident junk rows (content-free/tentative/deferred docs all produce
  "decisions"), confused decision/action buckets, mostly-missed owners.
- **R45.11 is now gated**: build only on a re-probe ≥85% with a stronger
  model/prompt. (Also fixed its stale "always `proposed`" → `unverified`.)
- **R45.14 filed (P2, 0.25d)**: `RECORD_PATTERNS` has no action-items /
  checkbox marker, so an action-list-only doc buckets as `reference` and
  parses nothing. Deterministic fix, no model needed.
- Corpus + runner preserved at `docs/probes/enrichment-corpus/` (the probe
  doc says `/tmp` — both exist; the repo copy is the durable one).

## 5. Wizard mode-first interim (R45.6 partial)

The welcome window pushed Ollama-for-answers with no mode choice. Interim
fix (frontend-only; backend readiness was already mode-aware): "Who writes
the answers?" selector on step 0, answer fields hidden + step relabeled
"Configure search" in agent mode, `agent_compose` treated as success in
step 3 (shared neutral hint). R45.6 row notes the interim; MCP-or-skill +
RAM-sized picker still need R45.4.

## 6. Resume here

1. **R45.10** (small, unblocked): demote + delete legacy entities, backup
   first, counts before/after. Removes the visible junk from 1.1.0's
   Decisions/Actions.
2. **R45.4 → R45.6 → R45.7** (unblocked): built-in embedder (nomic v1.5
   f16, per-OS features, download manager, Team prefetch — notes in the
   morning handover §3.2), then wizard finish + how-it-works.
3. **R45.14** (small): bucket-pattern fix; pairs naturally with R45.10.
4. In parallel: P0s **R30.1, R30.3, R36.2** (+ S2/S3 audit items).
5. R45.3 CI legs when pushing a branch anyway (MinGW, x86 mkl, Ollama CPU
   baseline — the last also settles the CPU-vs-CPU engine question).
