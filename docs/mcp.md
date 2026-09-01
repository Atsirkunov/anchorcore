# AnchorCore — Agent Connectivity via MCP (draft)

> Goal: let any AI harness (Claude Code, Codex, opencode, Cursor, …) connect
> to AnchorCore's memory — locally or against a centralized instance — and
> consume it with full provenance.
> Status: **B14.1 shipped (v1.0.9)**, **R12.5 local without UI shipped (1.0.12)** — Rust sidecar `rust/crates/anchorcore/src/bin/mcp.rs:1` stdio + HTTP `ask/search` (public_only gate, audited `component=mcp`) against `business-scale` without browser; B14.2 (HTTP transport) and B14.3 (write-back) planned.

---

## 1. The Problem We're Solving

Users increasingly run their own harnesses — Claude Code, Codex, opencode with
local or cloud models — but those harnesses start with **no memory**: every
session is amnesiac, decisions live in tickets/docs no agent reads, and
answers carry no provenance.

AnchorCore already solves that problem for the browser UI. The missing piece
is a standard way for external agents to reach in. We should not build one
integration per harness; we build **one adapter over a protocol everyone
speaks**.

## 2. Why MCP (Model Context Protocol)

- Claude Code, Codex, opencode, Cursor, and most agent frameworks all ship
  native MCP clients today.
- MCP gives us a clean tool/resources model that maps 1:1 onto our trust
  layer: every tool result can carry `source_ref`, confidence, and status.
- One server implementation = every harness works, now and for future tools.
- MCP is the emerging industry standard for agent↔data connectivity (the
  "context infrastructure" layer of our own thesis).

Alternative considered: a plain REST API + README. It works, but every harness
needs custom glue; MCP is the glue.

## 3. Tool Surface (v1 — read-only)

| Tool | Signature | Maps to |
|---|---|---|
| `ask` | `ask(question: str, project_id?: int, public_only?: bool)` → answer + citations | `AnswerEngine.ask` `POST /qa` / `POST /qa/public` `public_only` B30 |
| `search` | `search(query: str, k: int, project_id?: int, public_only?: bool)` → chunks/entities + scores | Vector + keyword retrieval `POST /qa/search {public_only}` `answer.rs:44` |
| `get_entity` | `get_entity(entity_id: int)` → entity + provenance | Entity graph |
| `get_source` | `get_source(source_id: int)` → source config (secrets masked) | Sources |
| `list_sources` | `list_sources()` → names, connectors, last sync | Sources |
| `memory_status` | `memory_status()` → counts, model availability, pending embeddings | `/system/status` data |

**`search` is the workhorse for harnesses.** Harnesses don't want a final
answer they can't inspect; they want raw context (entities, chunks, refs) to
feed their own reasoning loop. `ask` is for humans; `search` is for agents.

All results respect status filtering — disputed/stale entities excluded, same
as Q&A — so agents never cite contested facts.

## 4. Transports (two phases)

### 4.1 Local — stdio (v1, matches local-first thesis)

A sidecar process `anchorcore-mcp` (Python, `mcp` SDK) spawned by the harness:

```bash
# dev checkout (backend must be running first — ./start.ps1 / ./start.sh)
claude mcp add anchorcore -- python /path/to/backend/anchorcore_mcp.py
codex mcp add anchorcore -- python /path/to/backend/anchorcore_mcp.py
# opencode: "mcp" entry in opencode.json with local stdio command
```

The sidecar talks to the running backend over `127.0.0.1` (`ANCHOR_BACKEND_URL`,
default `http://127.0.0.1:8000`) — the backend stays the single owner of the
DB and Ollama, exactly like the browser UI does today. **Shipped as B14.1**:
`backend/anchorcore_mcp.py` + `backend/app/mcp/` (`server.py`, `tools.py`,
`backend.py`). Six read-only tools: `ask`, `search` (raw retrieval, the
harness workhorse), `get_entity`, `get_source`, `list_sources`,
`memory_status`. `search` maps to `POST /qa/search` (same hybrid pipeline as
`ask`, no LLM; status filters apply — agents never cite disputed/stale facts).
Tests: `backend/tests/test_mcp.py` (drives the real app via ASGITransport).

**Shipped as R12.5 (1.0.12):** Rust sidecar `rust/crates/anchorcore/src/bin/mcp.rs:1` mirrors Python — stdio JSON-RPC `initialize`/`tools/list`/`tools/call` + `ANCHOR_BACKEND_URL`/`ANCHOR_MCP_TOKEN` + `X-CSRF-Token` cache `OnceLock` for local `POST` when auth disabled (`R7.1`). `ask`/`search` now `public_only` (`tools_defs` `public_only` bool, `POST /qa/public` vs `POST /qa/search {public_only}` `answer.rs:44` `search_sync_inner` `public_source_ids` filter) + audited `audit_mcp()` `INSERT system_events (component='mcp', level='info', message='mcp tool ask/search')` (`GET /system/errors?component=mcp`). Verified without browser vs test corpus: `ask DVCA 2 cites`, `search MRGR public_only 1 vs 2` (filters `label=internal`), `system_events 4×`. `cargo build --bin anchorcore-mcp`.

Not yet packaged into the frozen apps (no PyInstaller entry) — the dev-checkout
command above is the documented path until B14.2 ships HTTP, which removes the
sidecar need entirely.

### 4.2 Centralized — streamable HTTP (v1.5+, the "centralized dataset")

MCP **streamable HTTP transport mounted on the same FastAPI app**, e.g.
`POST https://your-host/mcp`:

```bash
claude mcp add anchorcore --transport http https://your-host/mcp
```

- One process serves UI + REST API + MCP — no extra runtime.
- Bearer-token auth (see §5) — opt-in; disabled on localhost by default.
- This is the hook for the v2 team story: shared memory that every agent in
  the org reads from.

## 5. Auth & Auditing (centralized only)

| Concern | Approach |
|---|---|
| Transport | Bearer token in `Authorization` header (MCP HTTP spec) |
| Config | `ANCHOR_MCP_TOKEN` (secret, env/SecretStore, never in DB/logs) |
| Localhost | MCP endpoint open without token; binds 127.0.0.1 |
| Permission tiers (v2) | read-only key vs write-back key; per-team scoping |
| Audit | Every MCP call writes a `system_events` row (component `mcp`, user = token id) — "who asked what" is always answerable |
| Rate limiting | Simple per-token sliding window (v2; cheap, do it when tokens exist) |

## 6. Write-back (v2)

`ingest(text, source_ref, kind_hint)` — let harnesses feed decisions, docs,
and notes **back into** memory from their sessions. Harnesses become memory
producers, not just consumers. Risks to manage:

- **Trust**: ingested-from-agent items should be marked `unverified` + author
  `mcp:<token-id>` so the review UI owns confirmation — never auto-trusted.
- **Abuse**: write tokens only, rate-limited, per-team.

## 7. Architecture Changes

### New: `backend/app/mcp/` package

| File | Responsibility |
|---|---|
| `server.py` | Build MCP server (stdio + HTTP) from shared services |
| `tools.py` | Tool implementations — thin adapters over `AnswerEngine`, retrieval, models |
| `auth.py` | Token check + token→user mapping for HTTP transport |
| `audit.py` | system_events logging wrapper for MCP calls |

### Changed

- `backend/app/main.py` — mount MCP HTTP endpoint (`/mcp`) when enabled; construct `McpServer` with the same `AnswerEngine`/`Embedder`/`SecretStore` instances (no new engine logic).
- `backend/app/config.py` — new settings: `mcp_enabled: bool = False`, `mcp_token: str = ""` (env `ANCHOR_MCP_ENABLED`/`ANCHOR_MCP_TOKEN`), banner line.
- `backend/app/system_events.py` — no change (reused).
- `docs/product-plan.md` — backlog item B14 (see §9).

### Deliberately NOT changed

- DB schema (MCP is an adapter, not a new data model).
- AnswerEngine / retrieval internals (swap points stay as documented).
- Secrets: MCP token lives in SecretStore/env; masked in logs via existing redaction.

### Packaging impact

- macOS app ships the MCP sidecar binary next to the app (PyInstaller second entry point) — or, if HTTP transport is enabled by default on the local app, no sidecar needed at all: `claude mcp add --transport http http://localhost:8000/mcp`.
- Centralized deployment is the hosted v2 story — same code, env-driven.

## 8. Client Setup Examples

**Claude Code (local stdio):**
```bash
claude mcp add anchorcore -- /Applications/AnchorCore.app/Contents/Resources/anchorcore-mcp
```

**Claude Code (centralized):**
```bash
claude mcp add anchorcore --transport http https://your-host/mcp \
  --header "Authorization: Bearer <token>"
```

**Codex (centralized):**
```bash
codex mcp add anchorcore -- http https://your-host/mcp --auth-token <token>
```

**opencode** — add an `mcp` entry in `opencode.json` (remote URL or local command).

## 9. Milestones / Backlog (B14)

| # | Scope | Output | Effort |
|---|---|---|---|
| B14.1 | Local stdio MCP, read-only (`ask`, `search`, inspect) | ✅ **DONE (v1.0.9)** — sidecar + 6 tools + `/qa/search`; testers' harnesses see memory with citations | 1–2 days |
| B14.2 | HTTP transport + token auth + audit | Centralized "connect agents to shared memory" demo (config already added: `mcp_enabled`/`mcp_token`) | 1–2 days |
| B14.3 | Write-back `ingest` (unverified + review queue) | Agents feed memory; humans confirm | 1–2 days |
| B14.4 | Registry publishing (Claude Code marketplace etc.) | One-command install | 0.5 day + review |

## 10. Open Questions

- [ ] Stdio sidecar vs always-on local HTTP as the default local path?
- [ ] Token scoping model for v2 (per-user vs per-team, read/write tiers)?
- [ ] Do we cap `search` k and prompt budget to protect Ollama/context under agent load?
- [ ] Should `ask` be exposed at all, or is `search` enough for agents (answers for humans)?
- [ ] Naming: expose the product as `anchorcore` (brand) or `memory` (function) in MCP tool namespaces?

---

*Companion docs: [product-plan.md](./product-plan.md), [architecture.md](./architecture.md), [packaging.md](./packaging.md)*
