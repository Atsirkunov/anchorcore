---
name: ask-docs
description: Answer questions from the user's own or team's indexed documents, code, tickets and notes (AnchorCore, their local company memory) through its local REST API, read-only, with citations. Use whenever the user asks about their own material in any words, without naming AnchorCore — what did we decide about X and does it still hold, who owns X, what's open or overdue, where is X documented, what do our docs, specs, ADRs, notes or tickets say, how does our system handle X, is it implemented as documented — or invokes this skill with a question. Not for general knowledge. If AnchorCore MCP tools are available and working, use those instead.
---

<!-- anchorcore-skill: search (installs find and update this copy by this marker, whatever its folder is called) -->

# Ask docs (AnchorCore)

Read-only retrieval over the user's indexed sources. Never write, resync or
reclassify; that changes the user's index.

MCP-first, either/or: when anchorcore MCP tools are listed and answer, use
them and not this skill (both at once duplicates effort). This skill is for
when MCP can't be installed, can't start (no anchorcore tools after a
restart), or errors while the app still answers `GET /health`.

## Answering a question

People ask in their own words and rarely say "AnchorCore". Treat any
question about their own or their team's material as yours.

1. The question is whatever text came with the invocation
   (`/ask-docs what did we decide about refunds?`) or the user's
   message. If there is none, ask what to look up.
2. Connect (below). If a call fails, follow "When the app can't be reached"
   before anything else.
3. Route by shape: "what did we decide / is it still current" → GET
   /decisions?q=; "who owns / what's open / what's overdue" → GET /actions;
   everything else → POST /qa/search with short domain terms (Method, step 1).
4. Read before you state: fetch the hit's lines or section, not just the
   preview.
5. Answer in the user's terms and cite path plus lines or section for each
   claim. If nothing indexed matches, say so and name what you searched;
   never fill the gap from general knowledge as if it came from their
   documents.

## Connect

- Base URL: ANCHOR_BACKEND_URL env, else `http://127.0.0.1:<port>` with the
  `port` from `~/.anchorcore/runtime.json` (the running app writes it), else
  http://127.0.0.1:8000. Probe GET /health (expect `{"status":"ok"}`; a 503
  `{"status":"starting"}` means the app is upgrading its database — see
  below).
- Search is a POST: first GET /csrf, then send its csrf_token as
  X-CSRF-Token with Content-Type: application/json. (No CSRF header needed
  when you authenticate with a Bearer token instead.)
- If GET /auth/status says auth is enabled, log in as described in the
  anchorcore-setup skill.
- If a call fails, see "When the app can't be reached".

## When the app can't be reached

Tell the failures apart before you say anything:

- **AnchorCore is upgrading its database:** a 503 whose body says
  `"status":"starting"` (on /health) or "AnchorCore is upgrading its
  database". The app is running: tell the user "AnchorCore is upgrading its
  database (step N of M), I'll retry shortly", wait about 10 s and retry —
  large libraries take several minutes. Don't call it "not running". A 503
  with `"status":"failed"` means the upgrade failed: ask the user to open
  AnchorCore, which shows what happened.

- **AnchorCore isn't running:** connection refused, nothing listening on the
  port. Ask the user to open AnchorCore, then retry. Don't mention the
  sandbox.
- **Your sandbox blocked the call:** the harness reports a permission,
  network-policy or sandbox denial instead of a connection result. Ask once,
  for the narrowest access: "AnchorCore runs on this computer at
  127.0.0.1:<port>, and my sandbox blocks local network calls. Can I allow
  access to that address, or run this one request outside the sandbox?"
  Prefer allowing that address over leaving the sandbox. Never retry with a
  sandbox bypass the user didn't approve. If they say no, stop, give them
  the exact command to run themselves (in Claude Code:
  `! curl -s http://127.0.0.1:<port>/health`), and don't ask again this
  session.

If you can't tell which (a timeout with no denial message), say so and ask
whether the app is open.

## Calls

| Call | Use |
|---|---|
| POST /qa/search {"query","limit","include_content"} | ranked hits; filters: kind, symbol, path_prefix, env |
| GET /media/:asset_id?format=json | one image to look at: `{mime, w, h, size, data_base64}`; asset ids come from `images` on hits/citations. Without `format=json` you get the raw bytes |
| GET /items/:id | the whole file or document; text is in content (same as `text`); `assets` lists its images |
| GET /items/:id?lines=a-b | just lines a–b (1-based, inclusive; also `a` or `a-` to the end) plus `lines: {start, end, total}`; an end past the last line clamps, a bad range or a start past the end is 400 |
| GET /items/:id/outline | section tree with sizes and `line_start`/`line_end`; read it before fetching a large file (`depth=N` keeps the top levels) |
| GET /nodes/:id?include=text,parent,children,siblings | one section: `include` is any mix of `parent`, `children`, `siblings` (neighbour metadata) and `text` (the section's leaf text, its subsections included, in document order, with `text_truncated` + `text_chars`; `max_chars` caps it, default 32000) |
| GET /items?path_prefix=<relative prefix> | list items under a folder: metadata only (id, title, path), `total`, `limit`/`offset` |
| GET /decisions?status=current&q=<text> | decision log: record_id, status (`superseded` when a newer record supersedes it), current, owner, supersedes/superseded_by, provenance (item_id, path, chunk_id). status: accepted, superseded, rejected, deferred, proposed or `current` |
| GET /actions?owner=<name>&state=open | action tracker: task, owner, due, done, provenance. state: open (default), done or all; `overdue=true` keeps open items with an ISO due date before today; owner is case-insensitive |
| POST /qa {"question"} | gated alternative to search: in agent mode (`llm_mode: agent`) it returns `mode: "agent_compose"` — ranked citations with content, Sensitive gate applied, no server model; compose the answer yourself. In local mode the server's model answers (`composed`); `context_only`/`none` carry a `warning` — never quote that text as an answer |

A hit carries chunk_id, item_id, section_id, kind, summary (a match-centered
preview), breadcrumb, item_title, doc_type, env, a score and a path (the
section path, else the item's path; source_ref labels the chunk's origin).
Scores are only comparable within one query; the response's `retrieval` flag
tells you whether ranking was `hybrid` or `keyword_only`. Hits are pointers:
`line_start`/`line_end` give the hit's line range (0 when unknown). Fetch
that window with GET /items/:item_id?lines=a-b before you cite the lines. By default hits omit chunk bytes — set `include_content:true` (alias
for `fields:"content"`) to get them verbatim, and follow `next_cursor` for
further pages.

Reading big files: take the outline first, then read one section with
GET /nodes/:id?include=text, or a hit's lines with GET /items/:id?lines=a-b
(a hit's section_id is a node id). Line numbers count lines of the stored
text, the same text GET /items/:id returns. path_prefix (on /items and
search) is a relative path as /items lists it (`docs/`, `cfg/routing`) or an
absolute path under a folder source's root. Wildcards, `..`, a leading `./`
or `~`, and URLs get a 400 that names the expected form. They never get an
empty list. All of these reads take `public_only=true`, and anything outside
a public source — or marked Sensitive on one — then reads as 404. Whole-file
reads, line windows, section text, section/tag chunk lists and entity reads
from a Sensitive item answer 403 `refusal: "gated_provider"` under an
untrusted provider (multi-item lists drop the gated rows instead), like
search. Do not send `X-Anchor-Client: ui` (that header is the app's own UI
exemption, not an agent channel). MCP clients get the same reads as
`list_items`, `get_item` (with `lines`) and `get_outline` (with `node_id` +
`include`).

Images: add `images` to `fields` (e.g. `"fields":"content,images"`; `all`
includes them) and each hit lists the pictures on its passage —
`{asset_id, alt, caption, mime, w, h, url}`. Agent-mode /qa citations always
carry them. When the question is about something visual (a screen, a
diagram, a chart), fetch the image with GET /media/:asset_id?format=json and
look at it before answering; alt/caption are hints, not the picture. The
JSON form refuses images over 1.5 MB with 413 + `size` + the raw `url`;
Sensitive sources and Sensitive-marked documents answer 403 `refusal:
"gated_provider"` under an untrusted provider, like search. `url: null` means the bytes were not stored (over the
size cap) — say so instead of guessing. MCP clients get the same call as the
`get_asset` tool. A hit with `kind: "caption"` (path ending `caption by a
person`) is a person's description of an image no document links: cite it
as a caption by a person, with the image from its `images`. Captions are
written by people in the app — never write or suggest one yourself.

For "what did we decide about X, is it still current?", "who owns X?" or
"what's open/overdue for Alex?", call /decisions or /actions before search:
they read the records' own structure (Decision:, Status:, Owner:, Action
items), so owners, due dates and supersession come back exact. Both take
`source_ids` (comma list), `project_id`, `public_only`, `limit`, `offset`, and
apply the same scope and provider gate as /qa/search (`blocked`, `refusal:
"gated_provider"`). Rows are parsed, not reviewed: `review_status` stays
`unverified` until a person confirms one. Cite the provenance path, then fetch
the item if you need the surrounding text. MCP clients get the same calls as
the `decisions` and `actions` tools.

## Method

1. Search with short domain terms first ("direct debit cancellation"),
   then rephrase with synonyms (cancelled, withdrawn, reversed). Long natural
   questions often rank poorly.
2. Check where hits come from. Prefer ADRs, specs and source code over
   generated or agent-written reference notes, and live config over test or
   sandbox config. Say which you used.
3. Open the item or section before you state a fact; use the outline to jump
   to the right part.
4. A feature often spans services. When a document describes a flow, trace
   through each service that handles the event: search for the event, status
   or handler name in the consuming service and open the handler.
5. Before you say something is absent, search for its identifiers (names,
   codes, statuses from the documents) in code and config. Report a
   result as "not found by search", with lower confidence, never as proof.
6. Cite file and line or section for each claim, and mark confidence
   (high/medium/low) with one line on what would raise it.

## Asked "is it implemented as documented?"

List each documented step. Rate the code full, simplified (say what it does
instead) or absent, with location and confidence. Then rank the largest gaps
and list contradictions (names, codes, states) between docs and code.

## The user says the answer is wrong

Generate a bug report instead of guessing: `GET /system/report` (same base
URL and auth as above; `?index=1` for the previous query, 404 when nothing
was asked yet). It returns a redacted bundle for the last `/qa` or
`/qa/search` call — the query text, per-hit legs and ranks, scores, index
counts, versions, and the embedding family. It contains no document text,
titles, paths, settings, or secrets (titles and sources are pseudonyms plus
hashes), so the user can send it to support as-is after a glance at the
query text. Save it to a file when they ask for one; never paste chunk
content alongside it.
