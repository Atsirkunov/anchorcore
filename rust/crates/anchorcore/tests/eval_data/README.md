# R39.10 golden retrieval eval set

A dozen-document fixture corpus plus query→expected-doc pairs that pin
retrieval behavior while fusion is tuned (Phase 39). The runner reports
per-leg recall@5 — vector leg, keyword leg, TOC leaf — **and** fused
recall, so each retrieval leg can be measured independently before tuning
fusion.

- Runner: `rust/crates/anchorcore/tests/retrieval_eval.rs`
- Manifest: `eval.json` (docs + summaries + pairs)
- Corpus: `corpus/` (prose, code, and config docs)
- `corpus/support/`: file-shaped unit-test fixtures, NOT in the manifest and
  never indexed. Chunking/cleaning fixes pin there: the harness inserts file
  bytes directly (no cleaning, no real splitters), and `incident-02-midbody`
  currently has zero fused margin (any single-token corpus addition
  displaces it), which blocks new retrievability pins until fusion-side
  margin work lands.

## How to run

From `rust/`:

```sh
cargo test -p anchorcore --test retrieval_eval -- --nocapture
```

prints the per-leg recall table (each cell shows ✓rank or · for a miss,
plus a MEAN row). For the full ranked list behind every cell:

```sh
EVAL_VERBOSE=1 cargo test -p anchorcore --test retrieval_eval -- --nocapture
```

Plain `cargo test` runs the same set with output captured; the
assertions (fused recall 1.0 on every pair, the R39.2 floor-only
signature, manifest consistency, all-live fixture) gate CI.

## How to add pairs

1. Add the doc under `corpus/` with a realistic relative path (that path
   becomes the item `external_id`, folder-connector style).
2. Add a `docs` entry to `eval.json`: `id`, `file`, `title`, and a 1–2
   sentence `summary`. The summary is indexed as a `section_summary`
   chunk: it **must** contain the pair's key terms, or the TOC
   summary stage will not admit the section and the TOC leg will miss.
3. Add one or more `pairs` entries: `id`, natural-language `query`, and
   `expected` doc ids (more than one is allowed — e.g. the ADR-vs-
   lint-config pair — and scores partial credit).
4. Optional `heading` on a docs entry (R40.5): models production
   `embed_input(chunks.path, content)` — the leaf chunk's *embedding
   input* becomes `heading + "\n" + content` while FTS/snippet content
   stays raw. Use it for continuation-chunk pairs (query terms in the
   heading, absent from the bytes); docs without it embed bare content.
5. Optional `vocab` on a docs entry (R40.7, R45.13): `[[term, expansion], ...]`
   pairs seeded as glossary rows (origin `glossary`, as if a person marked
   a glossary file) for the doc's item. Use it for vocabulary pairs (query
   term matches a seeded term but appears in no body); the keyword leg
   must miss with the key removed. The initialism rule and agent-mode
   filtering are unit-tested in `vocab.rs`, not golden.
6. Run the table command and check the new row: fused must hit (it is
   asserted), and each leg should behave as the pair intends.

## Term discipline (read before writing docs)

The corpus is small on purpose, so every shared word matters:

- Repeat each pair's distinctive query terms 3–5× in its doc (title,
  headings, body, comments/docstrings, description fields). The vector
  leg uses deterministic hash embeddings, and cosine needs that overlap
  to clear the 0.2 floor.
- Keep distinctive terms out of other docs (`grep` the new term across
  `corpus/` before finishing). One shared word is harmless (BM25 still
  ranks the all-terms doc first); three shared words will crowd top-5.
- Exact token forms matter, not stems: `rollback` ≠ `roll back`,
  `calculated` ≠ `computed`, `limits` ≠ `limit`, `chose` ≠ `choose`.
  Write the query's exact tokens into the doc.
- Bare-code pairs (R39.2) invert the rule: the code appears **once** in
  a long doc and **never** in its summary, so vector and TOC miss while
  the keyword floor hits. Keep it that way.
- Never put env markers in fixture paths or file heads: path segments
  like `test/`, `sample/`, `example/`, `dev/` and head words like
  `mock`, `staging`, `fixture`, `sandbox`, `testdata` flip the doc to
  test env and `prefer_live_envs` will reorder it (`eval_fixture_is_live_env`
  fails loud if this happens). Exception (R43.27): `sandbox` and `staging`
  are weak head markers — one alone is ordinary prose and stays live;
  `adr-012-billing` deliberately carries a lone "sandbox" in its head as
  the regression pin (two distinct weak markers, or any strong one such
  as `mock`/`fixture`, still flip a doc).
- Queries must stay filename-free (no dotted tokens): a filename in the
  query triggers `filename_boost`, which bypasses the legs.

## What the numbers mean

- `vector`: the real scan leg (`cosine > 0.2` over `kind='document'`
  chunks) fed by hash embeddings — a lexical proxy, not semantics.
  It understates true vector recall on paraphrase queries; it measures
  leg plumbing, not model quality.
- `keyword`: the real FTS5 BM25 leg over leaf chunks.
- `toc`: the real `toc_search` (summary admit → fenced leaf search).
  Without a linked vec0 module the summary and leaf-vector stages
  decline and TOC runs keyword-gated — the same as production on
  bundled rusqlite.
- `fused`: `answer.rs::orchestrate` restated from its public pieces
  (TOC + discounted flat floor, else flat hybrid; production defaults),
  fusing exactly the reported leg outputs.
- Only `fused` (1.0 on every pair) and the R39.2 signature are
  asserted. The per-leg columns are the instrument: tune against them,
  then extend this set when a leg regresses in production.
