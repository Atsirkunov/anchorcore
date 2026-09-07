# Any JSON API → AnchorCore

Point AnchorCore at a JSON list API — tickets, issues, headless CMS, your own
backend. Map its fields once, preview, sync. No per-integration code.

## Setup (Sources → Add source → Generic REST API)

1. **Endpoint:** `base_url` + `list_path` (e.g. `https://api.example.com` +
   `/v1/tickets`; a full `http…` URL in `list_path` is used as-is). `GET` by
   default, `POST` if the API wants a body (`body_json` merges with page params).
2. **Auth:** none, Bearer [REDACTED] Basic (email + password), or a custom header
   (`header_name`, default `X-Api-Key`). Tokens and passwords are
   keychain-backed — stored as secrets, never in the DB. Re-enter the
   credential to preview; the stored secret stays hidden.
3. **Find the items:** `items_path` points at the array (`/data`, `/issues`).
   Leave it empty and the connector tries the body itself, then `/data`,
   `/items`, `/results`.
4. **Map the fields** with JSON pointers (`/a/b` verbatim, `a.b` shorthand):

   | Setting | Default | Notes |
   |---|---|---|
   | `map_id` | `/id` | Upsert key; empty ids get a stable content-hash fallback, so re-syncs update instead of duplicating |
   | `map_title` | `/title` | |
   | `map_text` | `/body` | Comma-separated pointers joined with blank lines, e.g. `description, comments` |
   | `map_author` | `/author` | |
   | `map_updated` | `/updated_at` | RFC3339 string or unix-epoch number; drives the incremental cursor |
5. **Preview, then Sync.** Preview fetches one page and shows the first 5
   mapped docs without persisting anything. When the mapping looks right,
   save and Sync — items become entities cited as `rest:<id>`
   (`ref_prefix` renames the prefix per source).

## Pagination & incremental sync

`page_mode`: `none`, `token` (next-token path, default `/nextPageToken`),
`page` (`page` + `per_page`, starting at `page_start`), or `offset`
(`offset` + `limit`). Param names are all overridable; `max_pages` bounds the
loop (default 20, up to 200). `since_param` sends the sync cursor back to the
API for incremental pulls.

## What you get downstream

REST items flow through the same pipeline as every other source: chunked,
classified, embedded, cited as `rest:<id>` in answers, and gated by the same
source labels and PII rules. Items with no title and no text are skipped.
