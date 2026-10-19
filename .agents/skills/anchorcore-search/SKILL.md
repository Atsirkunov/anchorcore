---
name: anchorcore-search
description: Read-only retrieval over the user's indexed AnchorCore sources through its local REST API when MCP is unavailable, blocked, or can't be installed — ranked search, whole-file fetch, section outlines. Never writes, resyncs, or reclassifies. Use when the user asks to search, look up, or answer from their AnchorCore knowledge base and MCP is not installed, blocked, or unreachable; or to check "is it implemented as documented" against indexed code and docs.
---

# AnchorCore search

Read-only retrieval over the user's indexed sources. Never write, resync or
reclassify; that changes the user's index.

MCP-first: use the anchorcore MCP tools when they work — this skill is the
fallback for when MCP can't be used (install/verify: anchorcore-setup skill,
step 5). Either/or, never both: with MCP tools *and* this skill active,
agents duplicate effort.

Use this skill's HTTP API when any of these holds:
- MCP can't be installed (the harness has no MCP support, or no way to add
  a server entry).
- MCP is blocked (the harness can't spawn the local `anchorcore mcp`
  process, or `tools/list` still shows no anchorcore tools after a restart).
- MCP is not reachable (its tools error or time out) while the app itself
  answers `GET /health` — talk to the app directly.
If MCP starts working later, drop this skill again.

## Connect

- Base URL: ANCHOR_BACKEND_URL env, else ~/.anchorcore/runtime.json, else
  http://127.0.0.1:8000. Probe GET /health (expect `{"status":"ok"}`).
- Search is a POST: first GET /csrf, then send its csrf_token as
  X-CSRF-Token with Content-Type: application/json. (No CSRF header needed
  when you authenticate with a Bearer token instead.)
- If GET /auth/status says auth is enabled, log in as described in the
  anchorcore-setup skill.
- If requests to 127.0.0.1 are blocked by your sandbox, ask the user to allow
  localhost access.

## Calls

| Call | Use |
|---|---|
| POST /qa/search {"query","limit","include_content"} | ranked hits; filters: kind, symbol, path_prefix, env |
| GET /items/:id | the whole file or document; text is in content (same as `text`) |
| GET /items/:id/outline | section tree with sizes; read it before fetching a large file |
| GET /nodes/:id?include=parent,children,siblings | section metadata + neighbours; fetch the item for the text |
| GET /items?path_prefix=<relative prefix> | list items under a folder |

A hit carries chunk_id, item_id, section_id, kind, summary (a match-centered
preview), breadcrumb, item_title, doc_type, env, a score and a path (the
section path, else the item's path; source_ref labels the chunk's origin).
Scores are only comparable within one query; the response's `retrieval` flag
tells you whether ranking was `hybrid` or `keyword_only`. Hits are pointers:
line numbers are not returned, so count lines in the fetched text if you cite
them. By default hits omit chunk bytes — set `include_content:true` (alias
for `fields:"content"`) to get them verbatim, and follow `next_cursor` for
further pages.

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
