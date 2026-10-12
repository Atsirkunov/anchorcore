# AnchorCore — Code-aware sections (plan)

> Status: shipped 2026-10-12 — R27.1–R27.4 landed (see `rust/BACKLOG.md` shipped log); acceptance script passes live (probe `:8241`, `PROBE_PASS`).

## What is already true (don't rebuild it)

- `doc_type=code` marks source files (extension fast path over `CODE_EXTS`, `extract.rs`), so the pipeline knows which items are code without an LLM call.
- `sections` table + `chunks.section_id/level/path`; `store_doc_chunks` (`pipeline.rs`) rebuilds sections per item on every sync.
- `toc_search` (`retrieval.rs:734`) is generic over sections: summary nodes → top-5 sections → leaf search within. New section shapes flow through with **zero retrieval changes**.
- Per-section summaries, tags, citation `path`, and the UI `Breadcrumb` are all generic over `sections.path`.
- `relationships.kind` is free text with no validation, and no production write path exists yet (only backup restore) — R27.4 adds the first one.
- `graph_expand` (`retrieval.rs:915`) walks *all* relationship kinds and scores unknown ones at `GRAPH_DEFAULT_WEIGHT` — a conflict link must be explicitly excluded or contradictions get pulled in as supporting evidence.
- Filing a dispute flips the entity to `disputed`, which Q&A excludes by default — so conflicts must NOT reuse disputes for their reason text (both sides stay visible, with a warning).

## Problem

`chunk_document_with_sections` only understands prose headings (`§`, numbered, ALL-CAPS). A code file yields one root section and arbitrary 1600-char paragraph packs that can split mid-function. Consequences: the summary-vector stage sees one summary per file (no pruning *within* a file), citations point at "file §N" with no symbol, and there is no file › symbol navigation.

## Target UX

Acceptance script (the demo that proves the phase):

1. Sync a folder containing a `sample.rs` (a struct, an `impl` with two methods, a free function).
2. `GET /sections?item_id=` shows `struct Item`, `impl Item › fn new`, `fn classify` with correct levels/parents/paths.
3. Ask "how does classify work" → the citation `path` names the symbol; the UI breadcrumb reads file › symbol.
4. Entities tab doc-card TOC lists the file's symbols.
5. Prose docs are byte-identical: existing section tests green, no behavior change for `doc_type != code`.
6. Flag prose decision #A as conflicting with code decision #B with a reason → both entity cards show the link and the note.
7. Ask about the topic → the citation on #A warns "contradicts #B".
8. Mark #B `stale` (existing PATCH, the human's "code wins / docs win" call) → the warning disappears and #B leaves retrieval.

## Design

- New `chunk_code_with_sections(text, filename)` in `chunking.rs` returning the same `(SectionMeta, ChunkMeta)` shape — `store_doc_chunks` only gains a dispatch on `ctx.doc_type == "code"` (the filename for family dispatch comes from `ClassifyCtx.external_id`, already stored).
- Family dispatch by extension (subsets of `CODE_EXTS`):
  - **python** (`py`, `pyi`): `^(class|def)\s+<name>` + indent stack, so methods nest under classes.
  - **brace family** (`rs`, `js/ts/...`, `go`, `java`, `cs`, `c/cpp/...`, …): symbol regex (`fn`/`function`/`class`/`struct`/`enum`/`trait`/`interface`/`impl` + modifiers) + brace-depth stack; depth 0–1 → level 1, deeper → level 2 (depth capped, never deeper).
  - **everything else code** (`sql`, shell, `ps1`, `css`, …): root-only, i.e. today's behavior, until a family is added — an explicit fallback list, not a gap.
- Guards: minified/generated files (avg line length > 2000 chars, or <10 newlines in >50KB) → root-only; symbol cap 500/file (overflow folds into a trailing root chunk); titles ≤80 chars like prose headings.
- Levels mirror prose: 1 = top-level symbol, 2 = nested (method/inner); preamble before the first symbol goes to a level-0 root section, exactly like prose.
- Chunking inside a symbol reuses `split_section` (blank-line packing, 1600 cap, `chunk_text` fallback) — a long function yields several chunks under one `section_id`, which section expansion already handles.
- No retrieval or UI work *for sections*: `toc_search`, summaries, tags, `Breadcrumb`, and citations are generic. R27.3 proves it live instead of building it twice.
- Conflict mechanics (R27.4): migration `15_conflict_note.sql` adds `relationships.note` (registered in `db.rs`, `try_add_column` mirror for advanced DBs); `POST /entities/{id}/conflicts {other_entity_id, note?}` validates distinct + both exist (404), rejects duplicates in either direction (409 `detail`, the codebase convention), inserts one `conflicts_with` row (confidence 1.0, note trimmed ≤1000 chars); `DELETE /entities/{id}/conflicts/{other_id}` unflags (404 when no link); `GET /entities/{id}/conflicts` lists `[{other_id, note, created_at}]` (`/related` stays untouched — it returns entities, not link metadata). One directed row suffices: `related` and all conflict reads query both directions.
- `graph_expand` skips `kind == "conflicts_with"` — contradictions must never enter as supporting evidence.
- Citation warning: `build_citations` gains a batched conflict lookup — entity chunks attribute via their own entity, document chunks (the common citation) via live entities on the same item; counterparts (or own-side entities) with status `stale` are excluded, since stale means resolved. `Citation.conflicts_with: Vec<i64>` + MCP `ask` passthrough; UI renders "contradicts #N" next to the citation and on entity cards (exact widget is the implementer's call within existing badge styles).
- Backup round-trips the note: `'note',r.note` joins the relationship `json_object` export + import insert, with a round-trip test (the column list is explicit — nothing is automatic).

## Plan (Phase 27 — `rust/BACKLOG.md`)

| # | Task | Priority | ~Est | DoD |
|---|---|---|---|---|
| R27.1 | Symbol splitter: `chunk_code_with_sections` + python/brace families + guards + fixture tests | P0 | 1d | `rs`/`py`/`ts`/`go` fixtures yield expected titles/levels/paths; minified + unknown-ext fixtures fall back to root-only; `cargo test` green |
| R27.2 | Pipeline dispatch on `doc_type == "code"` (+ backfill on re-sync) | P0 | 0.5d | sync a code fixture → symbol tree in `GET /sections`, chunks carry `section_id`; prose fixtures unchanged; re-sync rebuilds sections |
| R27.3 | Live proof + docs: Q&A cites symbol paths, UI breadcrumb check, plan marked shipped | P1 | 0.5d | acceptance script (Target UX) steps 1–5 pass against a real server; `AGENTS.md` gotchas updated if any |
| R27.4 | Decision conflicts: `conflicts_with` flag/unflag API + graph exclusion + citation warning + backup round-trip | P1 | 1d | flag/unflag round-trips with note, duplicate → 409; graph expansion skips conflicts (test); citation on a conflicted entity warns with the counterpart id, silent once the counterpart is `stale`; note survives export/import; `cargo test` + `npm run build` green; steps 6–8 pass live |

Order: (R27.1 → R27.2 → R27.3) ‖ R27.4 in parallel (~3d, two file-disjoint slices — R27.4 touches `entities.rs`/`retrieval.rs`/`answer.rs`/`backup.rs`/UI, never `chunking.rs`/`pipeline.rs`).

## Alternatives considered (rejected)

- **LLM sectioning** (ask the model for file structure per sync): rejected — a charged, non-deterministic call on every sync for structure a regex finds deterministically; also unavailable in degraded (no-model) runs, where sections must still build.
- **AST parsing via per-language grammar dependencies**: rejected — `CODE_EXTS` spans ~60 extensions across ~30 languages, so full parsing means a grammar dependency per language family to produce what amounts to symbol titles and nesting; regex + guards deliver that at a fraction of the weight. Revisit only if fixtures show regex precision below bar.
- **Reusing disputes for conflict reasons** (no migration): rejected — filing a dispute flips the entity to `disputed`, which Q&A excludes by default; a flagged conflict would hide one side instead of warning about both. The `note` column keeps both sides visible.

## Non-goals

- AST parsing; cross-file call or import graphs; a GitHub connector; LLM-based sectioning; automatic contradiction detection (candidates-in-review-queue at most, later, never auto-created); tracking symbol renames across syncs (sections rebuild per item — citations reference chunk content, which is stable enough).

## Risks

- Regex misfires on unusual code → conservative patterns, root fallback, one fixture test per supported language.
- Minified/generated files flooding the index → guards above (a later pass may skip `*.min.js` at the connector).
- Complexity budget → one small function per family plus a shared stack helper; `lizard -C 15` stays clean.
- Graph-expansion poisoning → `conflicts_with` excluded in `graph_expand` with a dedicated test (a contradiction must warn, never support).
- Silent backup loss → `note` added to the explicit relationship export/import lists with a round-trip test.

## Open questions (implementer's call, not blockers)

- Brace family: brace-depth vs indent for method nesting — decide in R27.1, covered by fixtures either way.
- Whether `impl` blocks become level-1 parents with methods as level-2 (recommended) or stay flat.
