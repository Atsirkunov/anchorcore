# Hierarchical TOC & Tag Taxonomy — Guide

> Shipped in 1.0.12. **TL;DR:** documents are section-aware now. Headings
> (`§434`, `4.2.1`, `Article 12`, ALL-CAPS) become a saved sections tree;
> each section gets a summary plus topic tags. Search goes coarse-to-fine —
> tags, then summaries, then passages — instead of scanning all 150k chunks.
> The UI shows a `Art 12 › 12.3` breadcrumb plus tag chips on every citation.

## 1. Why

Chunking used to be flat: split text into passages, rank all of them. That
works for hundreds of documents and falls over for thousands — every query
ranked the whole pile. The chunker already knew the document's structure
(it splits at headings); now that structure is saved and searched.

## 2. What gets stored

- **`sections`** — one row per heading: title, level, parent, full path
  (`Art 12 › 12.3`), a short summary, and the summary's embedding.
- **`chunks`** — every passage points at its section (`section_id`, `level`,
  `path`), so siblings and breadcrumbs are one query away.
- **`tags` + `chunk_tags`** — a topic taxonomy built as documents arrive. A
  proposed topic reuses an existing tag when close enough (similarity 0.82,
  tunable via `tag_reuse_threshold` in Settings), otherwise a new tag is
  created. Tags are lowercase-normalized with usage counts.

Migrations: `rust/crates/anchorcore/migrations/11_sections.sql`,
`12_tags.sql`; tag logic in `src/tags.rs`.

## 3. Ingestion

1. **Sections** (`src/chunking.rs`): headings detected (`§` forms, dotted
   numbers, article/annex/section words, ALL-CAPS lines) and nested with a
   stack into parent/child paths; passages pack up to 1600 chars per section.
2. **Summaries**: each section gets an extractive summary (first paragraphs,
   capped) plus one per document, stored as special chunks — these are what
   the coarse search pass reads.
3. **Tags** (`src/tags.rs`): keyword proposals per section, merged against
   existing tags at the 0.82 threshold (`count++`) or inserted new.
4. Re-syncing a document replaces its sections and chunks in one transaction;
   unchanged sections are hash-skipped like classifier windows.

PII-flagged content never reaches cloud embedding here either — the same
`is_pii` gate as the rest of the pipeline.

## 4. Retrieval — coarse-to-fine

`toc_search` (in `src/retrieval.rs`, tried before the flat hybrid search):

1. **Tag prune** — sections matching the query's tags (and the active
   project's sources).
2. **Summary search** — vector + keyword search over section/doc summaries
   only (~5k nodes instead of ~150k chunks) → top 5 sections.
3. **Leaf search** — vector + FTS search restricted to those sections,
   RRF-fused. Entity chunks bypass the section filter (they carry no
   section).

If the TOC pass comes back empty, retrieval falls back to the flat hybrid
search. Context expansion reads same-section siblings instead of ±1
neighbors; the 1–2 hop graph walk still runs after fusion.

Benchmarks: hierarchical search ~7ms vs ~40ms flat on a 30k-chunk probe
(5.6×); projected ~7ms vs ~200ms at 150k chunks (28×) — meets the
`p50 <50ms` bar.

## 5. API

- `GET /sections?item_id=&source_id=` — the tree for a document or source.
- `GET /sections/:id/chunks` — passages in one section.
- `GET /tags` — taxonomy ordered by use count.
- `GET /tags/:id/chunks` — passages carrying one tag.
- `GET/PUT /settings` — includes `tag_reuse_threshold` (default 0.82,
  overridable via `ANCHOR_TAG_REUSE_THRESHOLD`).

## 6. UI

- Entities and Ask citations render a `path` breadcrumb (`›`-separated) plus
  tag chips (`frontend/src/tabs/EntitiesTab.tsx`, `AskTab.tsx`).
- Settings has a TOC & Tags section with the reuse-threshold input.

## 7. How to verify

```bash
cargo run -p anchorcore -- --port 8123 --data-dir /tmp/ac-final
# add a folder source containing §434 / 4.2.1 style headings, then sync it
curl "http://127.0.0.1:8123/sections?item_id=1" | jq
curl http://127.0.0.1:8123/tags | jq
curl -X POST http://127.0.0.1:8123/qa/search -H "Content-Type: application/json" -d '{"query":"billing","k":5}' | jq '.hits[] | {path, tags, score}'
```

(CSRF: local `POST`s from curl need the `X-CSRF-Token` header — fetch it from
`GET /csrf` first. The MCP sidecar handles this automatically.)

## 8. Caveats

- `is_public_path` in `src/auth.rs` is an accepted high-complexity decision
  table — do not refactor (Phase 13).
- A few complexity warnings remain in `retrieval.rs` — follow-up, not blocking.
- Entity chunks have `NULL` section and bypass the section filter (keeps the
  disputed-entity tests green).

## 9. References

- `rust/BACKLOG.md` Phase 14 (`R14.1–R14.6`, done, 1.0.12)
- `docs/architecture.md` (system view; §4a retrieval detail)
- `README.md` ("Scale — hierarchical TOC" section)
