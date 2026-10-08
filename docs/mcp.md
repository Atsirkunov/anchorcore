# AnchorCore — Agent Connectivity via MCP

> Goal: let any AI harness (Claude Code, Codex, opencode, Cursor, …) connect
> to AnchorCore's memory — locally or against a shared instance — and
> consume it with full provenance.
> Status: **shipped** — Rust sidecar `anchorcore-mcp` (stdio, 6 read-only
> tools, `public_only` gate, per-call audit). HTTP transport and write-back
> are planned (B14.2/B14.3).

---

## 1. The Problem We're Solving

Users increasingly run their own harnesses — Claude Code, Codex, opencode with
local or cloud models — but those harnesses start with **no memory**: every
session is amnesiac, decisions live in tickets/docs no agent reads, and
answers carry no provenance.

AnchorCore already solves that problem for the browser UI. The missing piece
is a standard way for external agents to reach in. We don't build one
integration per harness; we build **one adapter over a protocol everyone
speaks**.

## 2. Why MCP (Model Context Protocol)

- Claude Code, Codex, opencode, Cursor, and most agent frameworks all ship
  native MCP clients today.
- MCP gives us a clean tool model that maps onto our trust layer: every tool
  result can carry `source_ref`, confidence, and status.
- One server implementation = every harness works, now and for future tools.
- MCP is the emerging industry standard for agent↔data connectivity.

Alternative considered: a plain REST API + README. It works, but every harness
needs custom glue; MCP is the glue.

## 3. Tool Surface (v1 — read-only)

| Tool | Parameters | Returns | Maps to |
|---|---|---|---|
| `ask` | `question` (required), `project_id?`, `public_only?` | composed answer + citations | `POST /qa`, `POST /qa/public` |
| `search` | `query` (required), `k` (default 8, max 50), `project_id?`, `public_only?` | verbatim cited chunks + scores | `POST /qa/search` |
| `get_entity` | `entity_id` | entity + provenance | Entity graph |
| `get_source` | `source_id` | source details, secrets masked | Sources |
| `list_sources` | — | names, connectors, last sync | Sources |
| `memory_status` | — | version, model availability, pending embeddings | `/system/status` |

**`search` is the workhorse for harnesses.** Harnesses don't want a final
answer they can't inspect; they want raw context (entities, chunks, refs) to
feed their own reasoning loop. `ask` is for humans; `search` is for agents.

All results respect status filtering — disputed/stale entities excluded, same
as Q&A — so agents never cite contested facts. With `public_only`, only
sources labeled public are searched (the share-safe surface).

## 4. Transports

### 4.1 Local — stdio (shipped)

The sidecar `anchorcore-mcp` ships with the app and is spawned by the harness.
The app must be running — the sidecar talks to it over `127.0.0.1`
(`ANCHOR_BACKEND_URL`, default `http://127.0.0.1:8000`). The app stays the
single owner of the DB, exactly like with the browser UI.

```bash
# macOS (app bundle)
claude mcp add anchorcore -- /Applications/AnchorCore.app/Contents/MacOS/anchorcore-mcp
# Windows (unzipped folder)
claude mcp add anchorcore -- C:\path\to\anchorcore-mcp.exe
# from a dev checkout
(cd rust && cargo build --bin anchorcore-mcp)  # → rust/target/debug/anchorcore-mcp
# opencode: "mcp" entry in opencode.json with local stdio command
```

Details: stdio JSON-RPC (`initialize`/`tools/list`/`tools/call`); every call
is logged to `system_events` (`component='mcp'`, visible under
`GET /system/errors?component=mcp`).

Legacy: the B14.1 Python sidecar (`backend/anchorcore_mcp.py` +
`backend/app/mcp/`, same 6 tools) is kept for hosting/conformance. New work
is Rust (`rust/crates/anchorcore/src/bin/mcp.rs`).

### 4.2 Centralized — streamable HTTP (planned, B14.2)

MCP **streamable HTTP transport mounted on the same Axum app**, e.g.
`POST https://your-host/mcp`:

```bash
claude mcp add anchorcore --transport http https://your-host/mcp
```

- One process serves UI + REST API + MCP — no extra runtime.
- Bearer-token auth (see §5) — opt-in; disabled on localhost by default.
- This is the hook for the v2 team story: shared memory that every agent in
  the org reads from.

## 5. Auth & Auditing

| Concern | Approach |
|---|---|
| Transport | Bearer token in `Authorization` header (MCP HTTP spec) |
| Config | `ANCHOR_MCP_TOKEN` (secret, env/SecretStore, never in DB/logs) |
| Localhost | stdio needs no token; binds 127.0.0.1 |
| Permission tiers (v2) | read-only key vs write-back key; per-team scoping |
| Audit | Every MCP call writes a `system_events` row (`component='mcp'`: tool, query sketch, hit counts) — "who asked what" is always answerable |
| Rate limiting | Simple per-token sliding window (v2; cheap, do it when tokens exist) |

## 6. Write-back (v2, planned)

`ingest(text, source_ref, kind_hint)` — let harnesses feed decisions, docs,
and notes **back into** memory from their sessions. Harnesses become memory
producers, not just consumers. Risks to manage:

- **Trust**: ingested-from-agent items should be marked `unverified` + author
  `mcp:<token-id>` so the review UI owns confirmation — never auto-trusted.
- **Abuse**: write tokens only, rate-limited, per-team.

## 7. Where the code lives

- `rust/crates/anchorcore/src/bin/mcp.rs` — the shipped sidecar: stdio
  server, the 6 tool definitions, HTTP calls into the app, audit logging.
- `backend/app/mcp/` + `backend/anchorcore_mcp.py` — legacy Python sidecar
  (hosting/conformance only).
- Config: `ANCHOR_BACKEND_URL` (default `http://127.0.0.1:8000`),
  `ANCHOR_MCP_TOKEN` (HTTP transport, planned).
- Planned: mount `/mcp` on the Axum app when the HTTP transport ships.

Deliberately unchanged by MCP: DB schema (it's an adapter, not a new data
model), retrieval internals, secrets handling (token lives in
SecretStore/env; masked in logs via existing redaction).

## 8. Client Setup Examples

**Claude Code (local stdio, works today):**
```bash
claude mcp add anchorcore -- /Applications/AnchorCore.app/Contents/MacOS/anchorcore-mcp
```

**Claude Code (centralized, planned — needs B14.2):**
```bash
claude mcp add anchorcore --transport http https://your-host/mcp \
  --header "Authorization: Bearer <token>"
```

**Codex (centralized, planned — needs B14.2):**
```bash
codex mcp add anchorcore -- http https://your-host/mcp --auth-token <token>
```

**opencode** — add an `mcp` entry in `opencode.json` (local command today,
remote URL once B14.2 ships).

## 9. Milestones

| # | Scope | Status |
|---|---|---|
| B14.1 | Local stdio MCP, read-only (Python sidecar + 6 tools + `/qa/search`) | ✅ DONE (v1.0.9, legacy) |
| R12.5 | Same surface as a Rust sidecar + `public_only` + audit | ✅ DONE (1.0.12, shipped) |
| B14.2 | HTTP transport + token auth + audit | Open |
| B14.3 | Write-back `ingest` (unverified + review queue) | Open |
| B14.4 | Registry publishing (Claude Code marketplace etc.) | Open |

## 10. Open Questions

- [ ] Stdio sidecar vs always-on local HTTP as the default local path?
- [ ] Token scoping model for v2 (per-user vs per-team, read/write tiers)?
- [ ] Do we cap `search` k and prompt budget to protect Ollama/context under agent load?
- [ ] Should `ask` be exposed at all, or is `search` enough for agents (answers for humans)?
- [ ] Naming: expose the product as `anchorcore` (brand) or `memory` (function) in MCP tool namespaces?

---

*Companion docs: [product-plan.md](./product-plan.md), [architecture.md](./architecture.md), [packaging.md](./packaging.md)*
