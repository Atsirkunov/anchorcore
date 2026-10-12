# AnchorCore — Code indexing redesign (plan)

> Status: approved 2026-10-12 — R33.1 signed off; R33.2 (code skips extraction) + R33.4 (config chunking) landed the same day. Open: R33.3, R33.5, R33.6. D2: entities fully out of retrieval (option a),
> with a who-knows audit as the gate (5–10 ownership questions per sample corpus against chunks-only
> retrieval in R33.3; option b is the fallback). D6: PRD folds into `reference`; bucket assignment stays
> fully deterministic (markers → `record`, everything else → `reference`); per-source manual override is
> the future escape hatch, not LLM detect.
> Owner reports: entity rows outrank content on 1.0.22–1.0.24 (R32 moved scores, not structure);
> entities are noise for code; chunks lack symbol/key paths; taxonomy (decision/document/…) is flaky both ways.

## What is already true (don't rebuild it)

- `doc_type=code` marks source files via the `is_code_filename` extension fast path (`extract.rs`) — no LLM call.
- Symbol sections ship (R27): `chunk_code_with_sections` (python-indent + brace families) returns `(SectionMeta, ChunkMeta)`
  via the shared `materialize` tail; `store_doc_chunks` dispatches on `doc_type == "code"`.
- `toc_search` is generic over section shapes: summary nodes prune, leaf search stays within top sections.
- Citations carry `path` (section breadcrumb), `item_id` (R32.2), path-derived `env` + `env` filter (R32.4).
- `GET /sections`, `GET /tags`, `GET /entities/{id}/context` (neighbour expansion) exist.
- Entities are deleted + rebuilt per item/window on re-ingest (`delete_stale_entities`) — stopping extraction
  for code purges code entities on the next re-sync/reclassify with no data migration.
- R32 evidence that patches move scores, not structure: filename boost (R32.1), post-fusion demotion (R32.3),
  `env` filter (R32.4) — each fixed the case that prompted it.
- Review/verify/dispute/conflict flows are correct for prose and stay untouched.

## Problem

1. **Entity rows outrank content by design, not by accident.** Leaf search is one RRF pool over
   `c.kind IN ('document','entity','distilled')` (`retrieval.rs`), and entity chunks bypass the section filter
   (`OR c.entity_id IS NOT NULL`, the R14.6 fix). A one-line summary matching two query terms beats the
   1600-char chunk matching the same two. Re-weighting cannot fix a shared pool.
2. **Entities are noise for code.** A 3B classifier paraphrasing code into decision/document/action/note
   cannot compete with the code itself, yet its rows compete in the same pool and fill the review queue.
3. **Chunks lack structure where it matters.** Configs (YAML/JSON/TOML) are `TEXT_EXTS` but not `CODE_EXTS`,
   so they go through the prose heading splitter and come out root-only — no key path. Search points at the
   right area but can't pinpoint the section; the best agent strategy is whole-item fetch.
4. **The taxonomy is flaky both ways.** 8 doc types × 4 entity kinds with per-type prompts, an 8-way LLM
   type-detect, and a regex rules fallback. The B26/R22.1 saga (note-only types) patched false positives;
   false negatives (real decisions missed, paraphrased beyond recognition) have no patch. Each new type
   prompt fixes one corpus and risks the others.

## Target UX

Acceptance script (the demo that proves the phase):

1. Sync a folder with Go sources, YAML configs, and prose docs (decisions + meetings).
2. Code items yield symbol sections (`GET /sections` shows file › symbol) and YAML items yield key sections
   (`server › tls › cert_file`); prose items unchanged.
3. Ask about a Go function → citations name the symbol; no entity paraphrase outranks the code.
4. `/qa/search` hits carry `path` + `symbol` + `kind` (code/config/test/doc) + parent item; filters work as
   filters (`kind=code`, `env=live`), not demotion hints.
5. Entities tab and review queue contain only prose entities; nothing from code to approve.
6. Directory listing over the indexed corpus + chunk-neighbour fetch + exact-string search work from MCP.
7. Prose decisions/actions on `sample/` + `sample-ai/`: fewer false positives than today, no real decision lost
   (before/after counts in the audit, owner eyeballs the diff).
8. Existing DBs: re-sync purges code entities; prose entities, conflicts, and review state survive.

## Design

- **D1 — Code is code: no entity extraction for `doc_type=code`.** The pipeline skips classify + distill +
  entity store for code items; it keeps extract → clean → symbol chunk → section summaries (pruning only) →
  embed. `doc_type=code` becomes a deterministic chunking signal (extension fast path), never an LLM
  judgement. Consequences, all intended: no code rows in Entities/review; `conflicts_with` becomes
  prose-vs-prose only (the flag API is unchanged); `who_knows` loses nothing (code entities rarely had
  owners); the `code` badge on chunks/citations/search hits stays — provenance, not extraction.
- **D2 — Chunks are the retrieval unit; entities are metadata.** Entity chunks leave the leaf pool:
  leaf search is `kind='document'` (+ summaries for pruning, never as citations). Entities stay reachable
  via lookup (`GET /entities/{id}`), `related`, `who_knows`, conflicts, and the review flows — they just
  stop competing with content in RRF. Two options for prose recall, owner picks one:
  (a) entities never enter retrieval (simplest; `ask` cites chunks, entity cards link from citations);
  (b) a dedicated entity-recall leg fires only for people/ownership queries (the existing `who_knows`
  trigger) and fuses separately, never in the content pool. Recommended: (a) first, (b) only if the
  acceptance script misses a real prose case.
- **D3 — Config chunking family (shipped R33.4).** Third `chunking.rs` family next to python/brace,
  keyed off a separate `CONFIG_EXTS` (yaml/yml/json/jsonc/toml — never in `CODE_EXTS`, so the doc_type
  stays a detected prose type and only the filename drives chunking + the extraction skip). As built:
  *every* key splits (leaves included — a one-line `cert_file:` section with path `server > tls >
  cert_file` pinpoints better than a folded leaf); lists/comments/`---` markers fold; TOML tables are
  flat sections under their full dotted name with every key a sub-section. Same `(SectionMeta,
  ChunkMeta)` return via `materialize` — zero retrieval changes, fixture tests per format. The YAML
  splitter was adopted from a concurrent worktree change; R33.4 generalized its dispatch (shared
  `extract::is_config_filename`), added JSON/TOML, and added the pipeline extraction + distill skip.
- **D4 — Metadata as stored filters.** `kind` (code/config/test/doc), `path`, `symbol`, `env`, parent item
  become stored per-chunk fields with query filters (`/qa/search` + MCP `search`), replacing the
  post-fusion demotion hints (R32.3 demotion and R32.1 filename boost retire once filters + leaf-only
  pool subsume them — delete the code, don't leave both).
- **D5 — Navigation primitives.** Directory listing over the indexed corpus (not the filesystem picker),
  fetch-a-chunk's-neighbours (generalise `entities/{id}/context` to chunks), exact-string search
  (non-ranked, alongside FTS/semantic) — all three in REST + MCP.
- **D6 — Prose taxonomy collapse.** Audit first: run `sample/` + `sample-ai/` through today's 8-type
  pipeline, record every decision/action with its source window, mark true/false positives with the owner.
  Then collapse doc types to behavioural buckets — the only behaviours keyed on type today are chunking
  (code vs prose), note-only suppression, and distill gating. Proposal: `code` (D1, deterministic),
  `record` (meeting/decision_log — the only types allowed to emit decision/action), `reference`
  (everything else — notes only, today's note-only semantics). One strict prompt per bucket; the 8-way
  LLM detect and the per-type prompt matrix go away. `DISTILL_DOC_TYPES` (currently duplicated in
  `classifier.rs` + `distill.rs`) collapses into the bucket definition. Measure before/after on both
  corpora; the audit, not a hunch, sets the bucket line.

## Plan (Phase 33 — `rust/BACKLOG.md`)

| # | Task | Priority | ~Est | DoD |
|---|---|---|---|---|
| R33.1 | Retrieval audit + this proposal sign-off | P0 | 0.5d | Owner picks D2 (a)/(b) and the D6 bucket line; decision recorded here. No code. |
| R33.2 | Code is code: skip extraction for `doc_type=code` | P0 | 1d | No classify/distill/entity-store for code; re-sync purges code entities; prose untouched; `cargo test` green, code-extraction tests rewritten to assert absence. |
| R33.3 | Leaf pool = document chunks; entities as metadata | P0 | 1d | Entity chunks leave leaf search (R14.6 bypass removed); entity lookup/related/who_knows/conflicts intact; acceptance steps 3–4 pass; R32.1/R32.3 patches retired if subsumed. |
| R33.4 | Config chunking family (YAML/JSON/TOML) | P1 | 1d | Key-path sections + fixtures per format; prose + code splitters byte-identical; `GET /sections` shows key paths. |
| R33.5 | Stored metadata filters + navigation primitives | P1 | 1.5d | `kind`/`symbol`/`env` stored + filterable in REST + MCP; dir listing, chunk neighbours, exact-string search; demotion-hint code deleted. |
| R33.6 | Prose taxonomy collapse | P1 | 1.5d | Audit table (before/after counts on both corpora, owner eyeballs diff); buckets replace the 8-type matrix; false positives down, no lost decision; prompts + detect code deleted. |

Order: R33.1 (sign-off) → R33.2 + R33.4 (parallel, file-disjoint: `pipeline.rs`/`classifier.rs` vs `chunking.rs`)
→ R33.3 → R33.5 → R33.6. R33.6 can parallelise with R33.5 (classifier vs retrieval/MCP surfaces).

## Alternatives considered (rejected)

- **Keep extracting, demote harder.** Rejected — R32 proved the pattern: each demotion fixes one case,
  the shared pool stays. The ranking problem is structural.
- **Collapse entity kinds instead of removing code extraction** (e.g. everything-from-code is `note`).
  Rejected — keeps the paraphrase competing with the source and the review queue full, for no retrieval value.
- **LLM sectioning for code/config.** Rejected (as in Phase 27) — charged, non-deterministic, unavailable
  degraded; regex + key parsing find the structure deterministically.
- **AST parsing per language.** Rejected (as in Phase 27) — ~30 languages for symbol titles + nesting;
  revisit only if fixtures show regex precision below bar.

## Non-goals

- Touching prose review/verify/dispute/conflict UX (see Phase 31, which proceeds independently).
- Cross-file call/import graphs; GitHub connector; automatic contradiction detection.
- Storage/index format changes (`vec_chunks`, FTS5, same SQLite file).
- Item fetch changes (it works; agents rely on it).

## Risks

- R33.3 changes prose recall shape (entity citations disappear) → acceptance step 7 + D2 option (b) fallback.
- R33.6 bucket line set wrong → the audit's before/after counts are the gate, not the implementer's taste.
- Deleting R32.1/R32.3 paths regresses the treasury-monorepo case → keep its repro as a test, prove the
  new shape passes it without the patches.
- Scope creep into review UX → Phase 31 owns that screen; this phase only changes what feeds it.

## Open questions (owner calls, not blockers)

- ~~D2: entities fully out of retrieval (a), or a people/ownership leg (b)?~~ Decided 2026-10-12: (a),
  with the who-knows audit as the gate (see Status).
- ~~D6: is `record` vs `reference` the right bucket line, or does `prd` need its own strict-decision bucket?~~
  Decided 2026-10-12: PRD folds into `reference`; deterministic assignment only.
- Should the purge of code entities on re-sync be silent, or surface a one-line "N code summaries removed" note?
- Exact-string search: literal substring only, or also regex?
