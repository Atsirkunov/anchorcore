-- R36.2: chunker generation per item. Every chunk write stamps the
-- generation that produced the item's chunks (`chunking::chunker_version`:
-- 1 = every chunker before R36.2, 2 = config rule-level sections). Existing
-- rows were all chunked by generation-1 splitters, so the column starts at 1;
-- `pipeline::rechunk_outdated_items` (run by `db::init_db` once the schema is
-- complete) rebuilds every item whose family has a newer generation from its
-- stored text, so one index never mixes old and new granularity. The rebuild
-- is Rust because the splitters are; the marker it keys on lives here.
ALTER TABLE ingested_items ADD COLUMN chunker_version INTEGER NOT NULL DEFAULT 1;
