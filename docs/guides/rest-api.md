# Any JSON API → AnchorCore

Point AnchorCore at a JSON list API — tickets, issues, headless CMS, your own
backend. Map its fields once, preview the result, sync. No per-integration code.

## Setup (Sources → Add source → Generic REST API)

1. **Endpoint:** `base_url` + `list_path` (e.g. `https://api.example.com` +
   `/v1/tickets`; a full `http…` URL in `list_path` is used as-is). Reads with
   `GET` by default, `POST` if the API wants a body.
2. **Auth:** none, Bearer token, Basic (email + password), or a custom header
   (default `X-Api-Key`). Tokens and passwords are stored in your OS keychain
   as secrets, never in the database.
3. **Find the items:** `items_path` points at the array (`/data`, `/issues`).
   Leave it empty and the connector tries the body itself, then `/data`,
   `/items`, `/results`.
4. **Map the fields** with JSON pointers (`/a/b` verbatim, `a.b` shorthand):

   | Setting | Default | Notes |
   |---|---|---|
   | `map_id` | `/id` | What identifies an item; re-syncs update instead of duplicating |
   | `map_title` | `/title` | |
   | `map_text` | `/body` | Comma-separated pointers are joined, e.g. `description, comments` |
   | `map_author` | `/author` | |
   | `map_updated` | `/updated_at` | Last-changed date; drives incremental sync |
5. **Preview, then Sync.** Preview fetches one page and shows the first 5
   mapped records without saving anything. When the mapping looks right,
   save and Sync — items appear in answers cited as `rest:<id>`.

## Pagination & incremental sync

`page_mode`: `none`, `token`, `page`, or `offset` — with all parameter names
overridable and `max_pages` bounding the loop (default 20, up to 200).
`since_param` sends the sync cursor back to the API for incremental pulls.

## What you get downstream

REST items flow through the same pipeline as every other source: read,
understood, cited as `rest:<id>` in answers, and gated by the same source
labels and privacy rules. Items with no title and no text are skipped.
