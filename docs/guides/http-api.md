# Search AnchorCore from your own app

AnchorCore runs a local HTTP API next to the app. Your scripts, internal tools
and agents call the same search the app and the MCP server use, and every hit
says where it came from: file, section and line range. No model is involved —
you get ranked passages with provenance and decide what to do with them.

## Connect

- **Base URL:** `http://127.0.0.1:8000` by default. The running app writes its
  actual port to `~/.anchorcore/runtime.json`. Check it with `GET /health`
  (`{"status":"ok"}`; a 503 `{"status":"starting"}` means the app is still
  upgrading its database).
- **Writes need a CSRF token.** Search is a `POST`: first `GET /csrf`, then send
  its `csrf_token` as `X-CSRF-Token` together with `Content-Type:
  application/json`, keeping the cookie from the first call.
- **Login only if auth is on.** A local install needs none. If `GET
  /auth/status` returns `{"enabled":true}` (the Team image, or a LAN setup),
  `POST /auth/login {"email","password"}` returns an `access_token`; send it as
  `Authorization: Bearer <token>` and skip the CSRF step.

```sh
BASE=http://127.0.0.1:8000
TOKEN=$(curl -s -c jar $BASE/csrf | python3 -c 'import sys,json;print(json.load(sys.stdin)["csrf_token"])')
curl -s -b jar -X POST $BASE/qa/search \
  -H 'Content-Type: application/json' -H "X-CSRF-Token: $TOKEN" \
  -d '{"query": "billing provider migration rollback", "limit": 5}'
```

## Search

`POST /qa/search {"query", "limit", "include_content"}` returns ranked hits.
Optional filters: `kind`, `symbol`, `path_prefix`, `env`, `project_id`.

Each hit carries `item_id`, `chunk_id`, `section_id`, `kind`, `item_title`,
`path` (the section path, else the item's path), `breadcrumb`, `summary` (a
preview centred on the match), `line_start`/`line_end` (0 when unknown), a
`score` and `provenance` — which search legs found it and at what rank. Scores
compare only within one query; the response's `retrieval` field says whether
ranking was `hybrid` or `keyword_only`.

Hits are pointers. By default they omit the passage bytes: set
`include_content: true` to get them verbatim. When more hits remain, the
response carries `next_cursor`; send it back as `cursor` for the next page.

Search words beat questions: short domain terms ("direct debit cancellation")
rank better than a long natural-language question.

## Read what a hit points at

| Call | Returns |
|---|---|
| `GET /items/:id` | the whole document; text in `content` |
| `GET /items/:id?lines=a-b` | lines a–b only (1-based, inclusive), plus `lines: {start, end, total}` |
| `GET /items/:id/outline` | the section tree with sizes and line ranges (`depth=N` keeps the top levels) |
| `GET /nodes/:id?include=text,parent,children,siblings` | one section; `text` is the section's own text, subsections included |
| `GET /items?path_prefix=docs/` | items under a folder: id, title, path (metadata only) |
| `GET /media/:asset_id?format=json` | one image as `{mime, w, h, size, data_base64}` |

## Decisions and action items

For "what did we decide about X?" and "who owns X?", the record endpoints are
more exact than search, because they read the documents' own structure
(`Decision:`, `Status:`, `Owner:`, action-item lists):

- `GET /decisions?status=current&q=<text>` — record id, status (`superseded`
  when a newer record replaces it), owner, what it supersedes, provenance.
- `GET /actions?owner=<name>&state=open` — task, owner, due date, done,
  provenance. `overdue=true` keeps open items past their due date.

Records are parsed, not reviewed: `review_status` stays `unverified` until a
person confirms one in the app.

## Sensitive sources

Callers other than the app's own UI see sources labelled **sensitive** (and
documents marked Sensitive) only after the person switches on **AI agents** in
Settings → Sensitive sources. Until then, lists drop those rows and single
reads answer `403` with `refusal: "gated_provider"`. Add `public_only=true` to
any read to see only sources labelled public.

## Agents

Coding agents can use this API through the search skill, or the same calls as
MCP tools: see [Agent connectivity (MCP)](../mcp.md).
