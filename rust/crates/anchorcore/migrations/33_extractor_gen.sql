-- R48.18: extractor generation per item. Every extraction stamps the
-- generation that produced the item's text (`extract::extractor_version`:
-- 1 = every extractor before R48.18, 2 = RTF after the R48.9 rewrite).
-- Existing rows start at 1; folder sync re-extracts a stale item even when
-- its stat fingerprint is unchanged, so an extractor fix reaches files that
-- were indexed before it (a same-bytes re-extract still hash-skips, so only
-- the fingerprint check is bypassed, never the content-hash authority).
ALTER TABLE ingested_items ADD COLUMN extractor_gen INTEGER NOT NULL DEFAULT 1;
