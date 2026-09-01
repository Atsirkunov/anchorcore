# Hierarchical TOC & Tag Taxonomy — Guide (Phase 14, shipped 1.0.12)

> **TL;DR:** Folder/Jira docs are now section-aware. Headings `§434`/`4.2.1`/`Article 12`/ALL-CAPS become a persisted `sections` tree; each section gets a `section_summary` + dynamic `tags` (`cosine>0.82` reuse). Retrieval is coarse-to-fine: tag prune → summary search (`~5k` sections) → leaf search `WHERE c.section_id IN (:top5)` — avoids flat `vector_search_scan` at `5k docs / ~150k chunks` (`scripts/build_business_corpus.py:12`). UI shows `path` breadcrumb `Art 12 › 12.3` + tag chips.

## 1. Why

Flat `chunk_text` `800/100` + `chunk_document` `1600` (`backend/app/chunking.py:49` / `rust/crates/anchorcore/src/chunking.rs:68`) was fast to build but flat to fetch. `chunk_document` derived `sections: list[str]` then discarded the tree — no `section_id`/`path` persisted, so hybrid RRF over `vec_chunks` vec0 (`k=top_k*4` `rust/crates/anchorcore/src/retrieval.rs:192`) + FTS5 `LIMIT top_k*4` had no hierarchical prune: `5k docs / ~150k chunks` still ranked 32 nearest flat chunks (`docs/architecture.md:79`). Phase 14 persists the tree the chunker already knew.

## 2. Schema

* `sections(id, item_id FK, parent_id FK NULL, level INT, title TEXT, path TEXT, chunk_range TEXT, summary TEXT, summary_embedding BLOB, summary_hash TEXT, created_at)` `rust/crates/anchorcore/migrations/11_sections.sql:1` `rust/crates/anchorcore/src/db.rs:107` `ensure_columns` + `migrations()` — index `ix_sections_item_id`, `ix_sections_parent_id`, `ix_chunks_section_id`.
* `chunks` adds `section_id INTEGER REFERENCES sections(id)`, `level INT`, `path TEXT` (`migrations/11_sections.sql`, `db.rs:123`).
* `tags(id, name UNIQUE, description TEXT, embedding BLOB, count INT)` + `chunk_tags(chunk_id, tag_id)` many:many `rust/crates/anchorcore/migrations/12_tags.sql:1` `db.rs:146` — `tags.rs:50` `threshold()` `0.82` `ANCHOR_TAG_REUSE_THRESHOLD` → `app_settings` `tag_reuse_threshold` (`settings.rs:7` `SETTING_KEYS`).

## 3. Ingestion

* `chunking.rs:68` `chunk_document_with_sections(full_text, base_ref)` returns `(Vec<SectionMeta>, Vec<ChunkMeta>)` — `heading_level()` `§` (level 1) / dotted `4.2.1` (level 2) / `Article 12` / ALL-CAPS (level 3) via stack; `level 0` root.
* `pipeline.rs:485` `store_doc_chunks` inserts `sections` then `chunks` with `section_id` in same txn; re-sync deletes `sections WHERE item_id` + `chunks WHERE item_id AND kind != distilled`.
* `pipeline.rs:408` `create_summaries` — per-section `extractive_section_summary` (first 2 paras, 300 tok) + doc `extractive_doc_summary`, stored as `chunks kind=section_summary`/`doc_summary` (`level/path` preserved), `hash-skip` via `sections.summary_hash` like `distill_hashes`, embedded via `embed_gated` (IDF `0.15` + `is_pii` gate).
* `pipeline.rs:553` `create_tags` — per-section `propose_tags` `tags.rs:26` `[\p{L}\p{N}]{4,}` `4` keywords, `ensure_tag` `deterministic_embed` cosine `>0.82` reuse (`count++`) else `INSERT` new tag; normalized lowercased.

## 4. Retrieval — coarse-to-fine

`answer.rs:244` `retrieve_sync` tries `retrieval.rs:836` `toc_search` before flat `hybrid`:

1. `matching_tag_sections` `chunk_tags`/`sections.path` + `project source_ids` → `Option<HashSet<section_id>>` tag prune.
2. `summary_vector_search_filtered` / `summary_keyword_search_filtered` `c.kind IN (section_summary,doc_summary)` `~5k` (`retrieval.rs:548`) — top 5 sections via `extract_top_sections`.
3. `leaf_vector_search_filtered` / `leaf_keyword_search_filtered` `c.kind IN (document,entity,distilled)` `WHERE c.section_id IN (:top5) OR c.entity_id IS NOT NULL` (`retrieval.rs:704` bypass for entity chunks `entity_id NULL` fix for `test_disputed_entity_stops_being_cited`) + `keyword_search_filtered` — RRF `rrf_fuse_multi` `1.0/(60+rank)`.

Falls back to flat `vector_search_filtered` / `keyword_search_filtered` + `vector_search_scan_filtered` `retrieval.rs:276` (now filtered by `section_id` when present) if `toc_search` empty. `expand_context` now `expand_by_section` `WHERE section_id = ?` siblings (`retrieval.rs:340`) vs `±1` for flat. Graph `graph_expand` `1–2 hops` still after `fuse_and_rank`.

Benchmark (`/tmp/bench_quick2.py` `30k ch / 32d`): flat scan `39.8ms` → hier `7.0ms` **5.6×**; extrap `150k` flat `200ms` → hier `7ms` **28×**; FTS `7.9ms → 2.3ms` **3.4×** — meets DoD `p50 <50ms` `rust/BACKLOG.md:190`.

## 5. API

* `GET /sections?item_id=&source_id=` `rust/crates/anchorcore/src/sections.rs:1` `list_handler` — `SELECT ... FROM sections WHERE item_id=? AND item_id IN (SELECT id FROM ingested_items WHERE source_id=?) ORDER BY level`.
* `GET /sections/:id/chunks` `chunks_handler` — `WHERE section_id = ?`.
* `GET /tags` `rust/crates/anchorcore/src/tags.rs:1` `list_handler` — `ORDER BY count DESC`.
* `GET /tags/:id/chunks` `chunks_handler` — `JOIN chunk_tags WHERE tag_id=?`.
* `GET /settings` `PUT /settings` now includes `tag_reuse_threshold` `settings.rs:7` `0.82` default (`env` `ANCHOR_TAG_REUSE_THRESHOLD` overrides).

## 6. UI

* `frontend/src/types.ts:Section/Tag/ChunkInfo/Citation` `path/tags` + `frontend/src/api.ts:sections/sectionChunks/tags/tagChunks`.
* `frontend/src/tabs/EntitiesTab.tsx:6` `Breadcrumb` (`path.split(" > ")` `›`) + `TagChips` (grouped view `sectionsByItem` + `allTags` fetch, flat view prefetch `sectionsByItem` per `item_id`).
* `frontend/src/tabs/AskTab.tsx:6` citation `Breadcrumb` + `TagChips`.
* `frontend/src/tabs/SettingsTab.tsx:268` TOC & Tags section `tag_reuse_threshold` input `PUT /settings`.

## 7. MCP (R12.5, same release)

`rust/crates/anchorcore/src/bin/mcp.rs:92` `tool_ask`/`tool_search` now `public_only` (`/qa/public` vs `/qa` + `POST /qa/search {public_only}` `answer.rs:44` `SearchRequest.public_only` → `search_sync_inner` `public_source_ids` filter) + `audit_mcp()` `INSERT system_events component=mcp` (`/system/errors?component=mcp`) — verified `ask DVCA 2 cites`, `search MRGR public_only 1 vs 2`.

## 8. How to verify

```bash
cargo run -p anchorcore -- --port 8123 --data-dir /tmp/ac-final
# folder with §434, 4.2.1 headings
curl -X POST http://127.0.0.1:8123/sources -H "X-CSRF-Token: $(curl -s http://127.0.0.1:8123/csrf | python3 -c "import sys,json; print(json.load(sys.stdin)['csrf_token'])")" -H "Content-Type: application/json" -d '{"connector":"folder","name":"t","config":{"path":"/tmp/src"},"label":"public"}'
curl -X POST http://127.0.0.1:8123/sources/1/sync -H "X-CSRF-Token: $CSRF"
curl "http://127.0.0.1:8123/sections?item_id=1" | jq
curl http://127.0.0.1:8123/tags | jq
curl -X POST http://127.0.0.1:8123/qa/search -H "X-CSRF-Token: $CSRF" -H "Content-Type: application/json" -d '{"query":"billing","k":5}' | jq '.hits[] | {path, tags, score}'
```

## 9. Caveats

* `is_public_path` `auth.rs:215` `CCN 22` accepted decision table — do not refactor (Phase 13).
* Lizard `retrieval.rs` `vec0_filtered 18` etc `8` warnings remain — follow-up.
* Entity chunks `entity_id IS NOT NULL` bypass `section_id` filter (`retrieval.rs:351` `hit_from_row11`) to keep `test_disputed_entity_stops_being_cited` green — entity chunks have `NULL` section.

## 10. References

* `rust/BACKLOG.md:186` Phase 14 `R14.1–R14.6` `done` `1.0.12` `75d51e4`
* `docs/architecture.md:3` + `docs/architecture.md:108` `4a`
* `README.md:101` Scale — hierarchical TOC
