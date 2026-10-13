# Benchmark: chunking / parent-child retrieval (R36.1)

Fixed question set + harness that every Phase 36 (parent-child retrieval)
build is measured against. The goal is boring comparability: same
questions, same corpora, same scoring, one command.

## Files

- `rust/scripts/bench_parent_child.py` — the harness (stdlib only).
- `rust/scripts/bench_questions.json` — the question set (13 entries:
  8 runnable against the repo corpora, 5 owner-run against private corpora).
- `rust/scripts/bench_retrieval.py` — the older R0.1 vec0-vs-scan latency
  gate. Different purpose (synthetic-corpus latency), kept as is.

## Method

Each corpus gets a **fresh server + fresh temp data dir** (no shared
state, no cross-corpus filename collisions — `people.md` exists in both
repo corpora). Per corpus the harness:

1. Starts a stock `anchorcore` binary on a free port (`--data-dir` under
   `/tmp/bench-ac-*`, `ANCHOR_SECRETS_NO_KEYRING=1`, browser closed).
2. Creates one `folder` source over the corpus dir and syncs it to
   `done` (polls `GET /sources/jobs/:id`).
3. Runs each question through `POST /qa/search` with `k=5` and
   `include_content: true`, timing every call.

Scores per question (top-5 hits):

| Column | Meaning |
|---|---|
| depth file@5 | Target file matched in top-5 (basename vs `item_title`, or relative-path substring in hit `path`/`source_ref`). |
| depth section@5 | Target section breadcrumb text matched in a hit's breadcrumb `path` — only `A > B`-style paths count, never file-path fallbacks (`-` when the entry has no section — file-level answers). |
| coverage@5 | **Every** `required_docs` entry appears in the top-5. Depth and breadth are scored SEPARATELY: a hit on the target file says nothing about coverage. |
| tokens (est) | `chars // 4` over returned hit contents. An ESTIMATE, not a tokenizer count — do not compare against real tokenizer numbers. |
| ms | Wall time of the `/qa/search` call. |
| retrieval | Backend reported by the API (`hybrid` vs `keyword_only`). |
| grep baseline | Manually recorded grep-and-read baseline (see below). `-` = not recorded yet. |

## How to run

From the repo root (build the binary first so the baseline measures your tree):

```sh
cargo build -p anchorcore          # from rust/
python rust/scripts/bench_parent_child.py --self-check   # validate questions + scoring, no server
python rust/scripts/bench_parent_child.py                # runnable subset (sample + sample-ai)
python rust/scripts/bench_parent_child.py --json-out /tmp/bench.json --keep-dir
```

Useful flags: `--binary` (a specific build to measure), `--k`,
`--corpus-dir NAME=PATH` (override/add a corpus dir),
`--include-owner-run` (with `--corpus-dir` for each private corpus),
`--keep-dir` (keep `/tmp/bench-ac-*` + `server.log` for debugging).

## How to read the score table

- Compare the **Totals** line across builds: `depth-file x/8, coverage y/8`.
  A parent-child change should move coverage first (more of the required
  docs surface) without regressing depth.
- `tokens (est)` guards context cost: a build that wins coverage at 3x
  tokens is a tradeoff, not a pure win.
- `retrieval` explains flakiness: `keyword_only` runs (no embedder) score
  differently from `hybrid`. Note the mode next to every table you publish.
- Per-question `hits` (chunk ids, kinds, paths, titles, scores) land in
  `--json-out` for audit — the `y/n` rollup never stands alone.
- Hit `path` is either a section breadcrumb (`A > B`) or the item's
  file-path fallback. Section needles match breadcrumbs only: matching
  them against file paths false-positives (observed: section "Tiers"
  vs file `d3-pricing-usage-tiers.md`).
- Baseline structure finding (2026-10-13, build 1.0.25): on the repo
  corpora, ingest yields root-only sections (one `level 0`, empty-path
  section per item; document chunks carry empty `path`, ~1 chunk per
  file) even though the sources contain `##` headings — so live top-5
  hits surface file-path fallbacks and `depth section` is a structural
  0/4, not a ranking failure. File/coverage metrics are the operative
  Phase 36 signal on these corpora; section scoring lights up on
  larger/sectioned docs (owner corpora) or once parent-child chunking
  lands. Whether root-only ingest of headed docs is intended is an
  open Rust-side question (see final report), not a harness bug.

## Grep-and-read baseline (manual — do NOT automate)

The `grep_baseline` column answers "could a human with grep have found
this?". Procedure per question:

1. `cd` into the corpus dir, fresh eyes, no AnchorCore.
2. Run grep candidates the way you naturally would
   (e.g. `grep -ril "rollback" .`), open the hits, read.
3. Record `y (<n> cmds)` when you reach the known answer, `n` when you
   give up or grep cannot express it (e.g. "why" questions whose answer
   vocabulary never appears in the query).

Fill the value into `bench_questions.json` (`grep_baseline` field) and
re-run — the harness prints whatever is recorded. The baseline is a
sanity anchor, not a gate: retrieval must beat grep on the "why/who"
questions and at least tie it on the keyword-obvious ones.

## Owner private-corpus entries

Entries `own-01..own-05` (`owner_run: true`) are skipped by default.
The corpora are owner-private and were never in the agent worktree:

- `own-01` (Acme types/routes in `routing-config.yml`) and
  `own-02` (reports / filename templates / columns) run against the
  treasury Go monorepo (`owner-treasury`).
- `own-03` (Atlas vs Beacon + Equities Launch Note) and `own-04` (funding methods)
  run against the private business docs (`owner-business`).
- `own-05` (direct debits) runs against the private PDFs (`owner-pdfs`).

`own-02..own-05` ship with blank `target`/`required_docs` — the
fine-grained file/section expectations can only be confirmed against
the real corpora. First-run procedure for the owner:

1. Map each corpus: `--corpus-dir owner-treasury=/path ...`.
2. Run with `--include-owner-run --json-out`.
3. Read the per-question `hits` in the JSON, confirm the known answer
   against the real files, fill in `target.file`/`target.section` and
   `required_docs`, record `grep_baseline` per the method above.
4. Re-run; that table is the private-corpus baseline. Keep it — later
   Phase 36 builds diff against it.

## Recorded baselines

Wave-1 merged tree (1.0.25 + R36.1 + R36.6, debug binary, hybrid,
2026-10-13): **depth-file 6/8, coverage 3/8**, section-depth 0/4
structural (sample corpora ingest root-only — no headings to split on).
Per-question: pay-01 y/y, pay-02 n/n, pay-03 n/n, pay-04 y(depth)/n,
ai-01 y/y, ai-02 y(depth)/n, ai-03 y/y, ai-04 y(depth)/n.

Noise note: coverage reads 3–5/8 across environments (agent worktree 5/8,
clean HEAD here 4/8, merged tree 3/8 twice) — `ai-02` flips on
embedding/classify nondeterminism, not on code changes. Treat ±2 coverage
as the noise band on this 8-question set; a Phase 36 delta counts when it
moves a question durably or shifts the total clear of 3–5. R36.6 re-ran
6/8 + 3/8 identical — additive API work is score-neutral as required.

Wave-2 merged tree (1.0.25 + R36.1 + R36.6 + R36.3 + R36.7 + integrator
funnel fixes, 2026-10-13): **depth-file 6/8, coverage 6/8** (identical
across 2 runs), first section-depth hit (`ai-01`). Covered: pay-01,
pay-04, ai-01, ai-02, ai-03, ai-04; still red: pay-02, pay-03 (red since
baseline — no regression). Durable deltas vs wave 1: pay-04/ai-04 newly
covered; ai-03 dipped to n on R36.7 alone (fine sections narrowed the TOC
top-5 funnel to one file) and recovered via the integrator's two fixes —
per-item diversity at the summary cut (fetch 15, admit ≤2/item, 10 total)
plus a discounted flat-hybrid recall floor unioned under TOC ranking, so
TOC can never underperform flat. Note: tokens/question FELL (fewer merged
hits, no duplicated `[continued]` bytes) — parent-grouping collapses
overlap, so down-with-held-recall is a win; per-hit bytes are UP (whole
sections with `[match]` marks).
