# Retrieval relevance tuning — cheat sheet

How chunk ranking works in AnchorCore and which knob to turn when answers
miss. Companion diagram: [retrieval-weights.svg](./retrieval-weights.svg).

## The 30-second model

Ranking is **deterministic code, not the LLM**. The model only embeds the
query, optionally rewrites follow-ups, and writes prose from already-ranked
hits. If a good chunk never surfaces, no model setting will save it — the
fix is in the pipeline below.

Pipeline in one line: vector + keyword legs → TOC prune → RRF fuse →
age decay → diversity/dedupe → live-first → top_k cut → filename boost.

## Symptom → knob

| Symptom | Likely cause | What to try |
|---|---|---|
| Right doc exists but never cited | `top_k` cut too tight, or TOC fence excluded its section | Raise `ANCHOR_TOP_K` (default 8); check with `POST /qa/search` whether the chunk appears at larger `k` |
| Keyword-obvious match loses to vague semantic hits | Keyword leg underweighted | Raise `ANCHOR_RETRIEVAL_KEYWORD_WEIGHT` (default 1.0); try 1.5–2.0 |
| Old doc should win but fresh trivia outranks it (or vice versa) | Age decay too strong/weak | `ANCHOR_RETRIEVAL_AGE_HALFLIFE_DAYS` (default 365). Larger = age matters less; `0` disables decay |
| One long doc crowds out everything else | Diversity cap too loose | Lower `ANCHOR_RETRIEVAL_MAX_PER_SOURCE` (default 3 per item) |
| Test/fixture/sample files outrank real docs | Live-first partition disabled or query matches markers | Ensure `ANCHOR_TEST_ENV_DEMOTE` is unset/`0.5`; `1.0` disables the partition. Queries naming test files are exempt by design |
| Query names a file but another file wins | Filename boost missed or grabbed the wrong file | Boost fires only on dotted tokens (`service.go`, not bare `service`); the file match is substring LIKE + first-id-wins, so it can grab the wrong file (R39.4). It runs after the cut and pulls below-cut chunks at `max + 1.0`, so when it fires it always wins; R43.1 threads project/public scope through the boost plus a final scope assert in `orchestrate_fusion`, so boosted hits can no longer leak across scopes |
| "Who owns X?" returns generic docs | Search has no ownership leg (the never-called `who_knows` leg was deleted in R45.9) | Ask the record listings instead: `GET /decisions?q=X` (owner per decision) or `GET /actions?owner=…` — MCP `decisions`/`actions`. Owners come from record structure (`Owner:`, `Name: task` action items) |
| Same content repeated across hits | Dedupe signature missed | Dedupe keys on `(item_id, first-160-chars normalized)` — cross-item copies are kept by design |
| Correct chunk missing even at large `k` | Binary gate, not a weight | Check: `stale`/`disputed` status, project scope, `public_only`, or `sensitive`/`pii` label vs untrusted provider |

## Knob reference

Env vars (effective at call time, no restart):

- `ANCHOR_TOP_K` (8) — final hits kept. Larger = more recall, noisier context.
- `ANCHOR_RETRIEVAL_KEYWORD_WEIGHT` (1.0) — RRF weight of the keyword leg
  vs vector leg at 1.0. Only affects the flat path; the TOC-internal fuse
  is fixed at 1.0/1.0 and the flat recall floor is fixed at 0.5/0.5.
- `ANCHOR_RETRIEVAL_AGE_HALFLIFE_DAYS` (365) — decay `0.5^(age/halflife)`.
- `ANCHOR_RETRIEVAL_MAX_PER_SOURCE` (3) — per-item cap; relaxed to `top_k`
  when fewer than 3 distinct items match.
- `ANCHOR_TEST_ENV_DEMOTE` (0.5) — any value in `[0,1)` keeps the
  live-first partition; `1.0` disables it. (The number itself no longer
  scales scores — ordering-only since R33.3.)
- `ANCHOR_KEYWORD_SLOTS` (2) - reserved tail slots for top floor-keyword
  hits locked out by fusion; `0` disables (R39.2).
- `ANCHOR_TYPE_SLOTS` (2) - reserved tail slots for the best below-cut
  hit of each doc kind (`doc_type`) missing from the cut, so sparse
  kinds survive ADR/code crowding; `0` disables (R40.4). Eviction
  prefers duplicated-kind tail (singleton kinds survive); when
  singletons cover the whole tail, keyword slots win the collision.
- `ANCHOR_MIN_RELEVANCE_COSINE` (0.50) - vector-cosine floor for evidence
  with no BM25 match; below it on every hit returns empty (R39.3).
- `ANCHOR_AUTHORITY_TIERS` (on) - set `0`/`false` to disable the (env, tier) ordering partition (R39.7).
- `ANCHOR_DECAY_FLOOR` (0.8) - age decay never drops below this; `0`
  restores raw decay (R39.6).
- `ANCHOR_QA_EXCLUDE_DISPUTED` (true) — set `0`/`false` to include
  `disputed` chunks.

Code constants (need a rebuild; change only with a failing test in hand):

- `RRF_K = 60` — RRF smoothing. Higher = flatter rank curve (top ranks
  matter less). Rarely the right fix.
- `CANDIDATE_MULT = 10` — legs fetch `top_k × 10`; live-first and the cut
  can only pick what was fetched.
- `TOC_SUMMARY_FETCH = 15` — summary candidates before top-section prune.
- `GRAPH_KIND_WEIGHTS` — currently unused by ask (graph leg removed from
  the ask path; entities never enter retrieval).

## How to verify a change

1. `POST /qa/search` with the failing query at increasing `k` — if the
   chunk appears at large `k` but not at 8, it's the cut/fusion; if it
   never appears, it's a gate or the TOC fence.
2. `mode: "exact"` in the same endpoint — unranked substring match. If
   exact finds it but ranked doesn't, the chunk is gated or pruned, not
   "low-scored".
3. Change one knob at a time; re-run the same query set. Relevance has no
   unit test for your corpus — your failing queries are the test.

## Things that won't help

- **Answer model / temperature** — the LLM writes prose from frozen hits;
  it cannot promote a chunk (there is no LLM reranker).
- **Embedding model swap** — changes vector-leg inputs, not weights; try
  it only when keyword search already finds the chunk but vector doesn't.
- **Score demotion before RRF** — RRF fuses by rank and ignores scores, so
  pre-fusion multipliers change displayed scores without reordering.
  Demote after fusion or partition before the cut (that's what live-first
  does).

## Shipped fixes (Phases 39+40.1-40.3, 2026-10-19) + remaining issues

- Floor-only keyword lockout: fixed by R39.2. `ANCHOR_KEYWORD_SLOTS`
  (default 2, `0` disables) reserves tail slots for top floor-keyword hits
  that fusion would drop; live-first re-partitions after the insert.
- Opaque single score: fixed by R39.1. Every search hit and citation now
  carries `provenance` (contributing legs + 1-based ranks), `decay` (applied
  factor), and `boosted` (surfaced by filename boost, bypassed ranking).
- No "nothing relevant": fixed by R39.3. `orchestrate` returns empty
  unless some hit has a BM25 match, a boost, or cosine >=
  `ANCHOR_MIN_RELEVANCE_COSINE` (default 0.50, calibrated on
  nomic-embed-text x `sample/`); `ask` answers "not found" (`mode: "none"`).
- Unbounded age decay: floored by R39.6. `ANCHOR_DECAY_FLOOR` (default
  0.8, clamped [0,1]) - a year-old chunk keeps 0.8 instead of 0.5.
- Ghost sections from contents pages: fixed by R39.9, sparse-hardened by
  R40.3 (sliding windows catch blank-separated entries; dot leaders don't
  count toward the length cap). Both apply at chunk time only - already
  indexed items keep their ghosts until a force re-sync rebuilds them.
- Boost file matching: fixed by R39.4. Exact basename beats substring,
  shortest path wins the rest (`best_item_for_name`); version-like tokens
  (`v1.2`) never fire. Boost scoring stays exactly `max + 1.0`.
- Boost status hole: fixed by R39.5. The `qa_exclude_disputed` flag threads
  from `orchestrate` into the boost (no hardcoded value), the stale/disputed
  filter runs before the top-k cut, and status re-filters after
  `parent_return`.
- Suspect-PDF fence: fixed by R39.8. `toc_search` splices flat-leg hits for
  `suspect` items after the fenced legs; clean corpora skip the extra legs.
- No source-authority signal: fixed by R39.7. `prefer_authority_tiers`
  stable-sorts by (env, tier) after live-first (tiers refine within env
  classes; filename mentions exempt; `ANCHOR_AUTHORITY_TIERS=0` disables):
  Spec (adr/rfc/decision dirs, adr-/rfc- names) < Code (source
  extensions) < Config (config extensions, Dockerfile/Makefile) < Prose
  (default) < Generated (vendor/generated/build dirs, lint names,
  lockfiles, .min/.bundle). Lowest authority wins on overlap
  (`vendor/spec.md` reads Generated). Watch item: prose sits below config
  by design (BM25 OR-queries otherwise bury exact config answers) -
  flip via the kill knob if runbooks lose wrongly.
- Eval mirror drift: fixed by R39.12. `retrieval::orchestrate_fusion` is
  the single fusion path for ask, search, and the eval; the mirror is
  deleted.
- Ignored `limit`: fixed by R40.1. `limit` pages over the `k`-sized
  candidate list, so a bare `limit: 50` now widens retrieval past the
  default k=8 (1.0.29 returned 7 hits for any limit without `k`).
- Mixed score scales: fixed by R40.2. Keyword-slot hits rescored into RRF
  range (the floor leg's 0.5/(60+rank) share) instead of raw BM25 (~14
  next to ~0.01); provenance keeps the real keyword rank.
- Heading-prefixed vectors: since R38.2 every embed path
  (`embed_chunks`, backup re-embed, scheduler backfill) embeds
  `embed_input(chunks.path, content)` — breadcrumb + text — on every
  sectioned doc type, not just markdown. R40.5 pinned it: the prose
  splitter no longer emits leading-`" > "` paths, and the golden pair
  `rfp-escrow-01` proves a continuation chunk (heading in path, not in
  bytes) clears the vector leg only with the prefix. Backfill story:
  vectors embedded before 1.0.28 are bare and stay bare — they refresh
  only on force re-sync (re-chunk NULLs embeddings, re-embed prefixes
  them); the scheduler backfill fills NULLs with prefixed input. No
  silent mass re-embed (recorded 2026-10-19, `rust/docs/decisions.md`).
- Nomic prefixes (R43.6): `nomic-embed-text` inputs carry
  `search_query:` (queries) / `search_document:` (documents + scheduler
  backfill) when the model name contains `nomic`; other models are
  untouched. Same no-mass-re-embed rule as heading prefixes — old rows
  stay unprefixed until force re-sync (recorded 2026-10-21,
  `rust/docs/decisions.md`). Embedding calls now error on vector-count
  mismatch instead of silently truncating (middle drops used to shift
  later vectors onto the wrong chunks). The golden eval cannot see
  prefix effects (hash embeddings), so this ships on unit tests, not
  the eval.
- Vector provenance (R45.5, migration 22, `embed_fp.rs`): every chunk
  vector is stamped `chunks.embed_fp = provider:model:dim` at write time
  (`Embedder::sync_vec`, which every embed path already calls), and each
  query vector records its fingerprint in `embed_state` before retrieval
  runs. The vector legs (scan and vec0) compare only stored vectors in
  the query's *family* + dim; everything else (another model, unstamped
  vectors from old archives or raw writes) is keyword-only until re-embedded.
  Families: unknown models are their own family; Ollama `nomic-embed-text`
  and the built-in nomic v1.5 share one (probe parity 1.00000), so a
  provider switch between them re-embeds nothing. A model switch is
  noticed by the next query: it records a `system_events` warning and wakes
  the scheduler backfill, which re-embeds stale vectors (NULL, unstamped,
  or out-of-family; label-gated like before) in id-cursor batches. Old
  vectors are overwritten, so switching back re-embeds again. If the
  fingerprint can't be recorded, that query runs keyword-only rather than
  against a stale filter. Migration 22 stamps pre-existing vectors
  `ollama:<DB embed_model or nomic-embed-text>:dim`; an env-only
  `ANCHOR_EMBED_MODEL` override costs one re-embed after upgrade. Backup
  archives now carry `embed_fp` on `chunk_embedding` records; older
  archives restore unstamped (re-embedded by the backfill).
- Corpus vocabulary (R40.7): FTS query terms expand through the
  `vocabulary` table (migration 20) on top of the R40.6 curated map.
  Tier-1 rules (parenthetical acronyms, glossary lines) are active
  immediately, no model needed; Tier-2 LLM pairs (`defined_terms` on the
  classify call, zero extra calls) stay `proposed` until a second
  independent item corroborates, a rule row confirms, or R40.8 review
  approves — single witnesses never expand. Lookup is project-scoped
  (`source_ids`); expansion adds terms only, chunk label/status gates
  still apply downstream. Known gap: `vocabulary` is not in backup
  archives yet — restore + force re-sync rebuilds it (rules fully, LLM
  pairs re-proposed).
- Vocabulary approval (R40.8): the third activation path. `GET
  /review/vocabulary` lists proposed pairs (term ↔ expansion, witness
  doc, origin, witness count; rule-touched pairs never list);
  `POST /review/vocabulary/{id} {decision: approve|reject}` decides the
  whole pair family. Approve activates, reject hides the pair for good
  (survives re-sync via the R40.7 store skip). The Entities tab folds the
  queue inline ("Words to confirm") next to the doubles strip — no
  separate Review surface (R31.2 folded it).
- Still open: suspect detector duplicated pending reunification (R39.13,
  P3); cross-encoder reranker deferred (R39.11, owner call).

## Code pointers

- `rust/crates/anchorcore/src/retrieval.rs` — `rrf_fuse_multi`,
  `toc_search`, `fuse_and_rank`, `prefer_live_envs`, `filename_boost`,
  `age_decay`
- `rust/crates/anchorcore/src/records_api.rs` — decision/action listings
  (R45.9; same scope + gate as search, not a retrieval leg)
- `rust/crates/anchorcore/src/answer.rs` — `orchestrate` (TOC ∪ flat
  floor), `RetrievalConfig::from_env*`, `generate_answer`
