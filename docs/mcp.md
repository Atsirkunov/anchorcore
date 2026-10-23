# AnchorCore — Agent Connectivity via MCP

> Goal: let any AI harness (Claude Code, Codex, opencode, Cursor, …) connect
> to AnchorCore's memory — locally or against a shared instance — and
> consume it with full provenance.
> Status: **shipped** — Rust MCP mode `anchorcore mcp` (single binary, R20.8;
> stdio, 6 read-only tools, `public_only` gate, ask/search audit). HTTP
> transport and write-back are planned (B14.2/B14.3).

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
| `ask` | `question` (required), `project_id?`, `public_only?`, `include_content?` | composed answer (local mode) or `agent_compose` evidence (agent mode, §3.2) + citations with provenance (item id + file path + title + env class on every citation; full chunk bytes on opt-in — always in agent mode — capped at 4000 chars with `content_truncated`) + `mode`/`warning` flags (§3.1) | `POST /qa`, `POST /qa/public` |
| `search` | `query` (required), `k` (default 8, max 50), `project_id?`, `public_only?`, `env?` (`live`/`test`) | cited chunks with provenance (item id/title/path, env class, source ref, score); summaries/snippets window on the match; test fixtures rank below live hits unless the query names them; long chunks capped at 4000 chars with `content_truncated` flagged; `images` (asset refs on the passage, R46.10 — fetch with `get_asset`) | `POST /qa/search` |
| `get_asset` | `asset_id` (from `images` on a hit/citation) | the image as an MCP image block + metadata (`mime`, `w`, `h`, `size`, `url`); SVG returns as markup text; over 1.5 MB → error with size + raw url; sensitive sources gated like `search` (R46.10) | `GET /media/:uid?format=json` |
| `get_entity` | `entity_id` | entity + parent item/chunk links (`item_id`, `chunk_id`) + legacy item PII flag (deprecated) | Entity graph |
| `get_source` | `source_id` | source details (no config/secrets) | Sources |
| `list_sources` | — | names, connectors, last sync | Sources |
| `memory_status` | — | version, model availability, pending embeddings | `/system/status` |
| `decisions` | `q?`, `status?` (`accepted`/`superseded`/`rejected`/`deferred`/`proposed`/`current`), `source_ids?`, `project_id?`, `public_only?`, `limit?`, `offset?` | decision log parsed from record structure: `record_id`, effective `status`, `current`, `owner`, `supersedes`/`superseded_by` (inverse links resolved), `provenance` (item id, source id, path, title, chunk id) + `total`/`blocked`/`refusal` | `GET /decisions` (R45.9) |
| `actions` | `owner?`, `state?` (`open` default/`done`/`all`), `overdue?`, `q?`, `source_ids?`, `project_id?`, `public_only?`, `limit?`, `offset?` | action items: `task`, `owner`, `due`, `done`, `provenance` + `total`/`blocked`/`refusal`; ISO-dated first | `GET /actions` (R45.9) |

**`decisions`/`actions` are REST first** (R45.9): the MCP tools forward only
the REST query keys (a test pins it), so an agent on the `anchorcore-search`
skill gets the same answers without MCP. Scope and gating mirror `search` for
the same caller: `project_id` + `public_only` resolve the same way, stale (and
disputed, per `ANCHOR_QA_EXCLUDE_DISPUTED`) rows drop, and an untrusted answer
provider withholds sensitive/pii-source rows (`blocked`, plus `refusal:
"gated_provider"` when nothing survives).

**`search` is the workhorse for harnesses.** Harnesses don't want a final
answer they can't inspect; they want raw context (entities, chunks, refs) to
feed their own reasoning loop. `ask` is for humans; `search` is for agents.

All results respect status filtering — disputed/stale entities excluded, same
as Q&A — so agents never cite contested facts. With `public_only`, only
sources labeled public are searched (the share-safe surface).

### 3.1 Bring your own model (BYOM) — the answer contract (R23.4)

AnchorCore never picks a model behind the user's back. The contract:

- **Harness agents prefer `search` + their own model.** `search` returns
  cited chunks (verbatim bytes on `fields: all`, capped at 4000 chars with
  `content_truncated`); the harness composes the answer with whatever model
  the user already runs. No server-side model is involved at all.
- **`ask` uses the server's configured answer model — the user's own.**
  Local Ollama or the user's own OpenAI-compatible endpoint (Settings →
  Answer model, or the onboarding wizard's "Configure model" step). There
  is no silent fallback to a model the user did not choose.
- **Degraded answers stay visible, never silent.** Every `ask` result
  carries `mode` (`composed` | `agent_compose` | `context_only` | `none`)
  and — whenever `mode` is `context_only` or `none` — a plain-language
  `warning` (`agent_compose` is by design, §3.2, and never warns). Agents must not quote
  warned text as an answer: use the citations, or fall back to `search` +
  their own model. The Ask tab renders the same warning as an amber
  banner, so humans see it too.
- **No Ollama fallback for answers.** `answer_base_url` defaults to
  `https://api.openai.com/v1` — unlike classifier/embed, it never falls
  back to Ollama. A local-only setup must set
  `http://localhost:11434/v1` explicitly, or answers stay `context_only`
  with the warning shown. The Settings "Run compose probe" button (and
  the wizard's step-3 question) verifies the endpoint end to end: only
  `mode: "composed"` means answers really work.

### 3.2 Agentic vs local LLM — `llm_mode` (R45.2)

One setting (`PUT /settings {"llm_mode":"agent"|"local"}`, Settings → "Who
writes the answers?"; default `local`, so upgrades keep today's behavior).
`GET /system/status` reports it as `llm_mode` (and `answer.mode`).

- **`agent`** — the user's AI agent writes answers, over **MCP or the REST
  `anchorcore-search` skill** (equal channels; no feature is MCP-only).
  AnchorCore runs no chat model on the answer path: `ask` / `POST /qa` /
  `POST /qa/public` do scope + `public_only` + the label/PII gate + fusion,
  skip the follow-up rewrite and generation, and return
  `mode: "agent_compose"` — `answer` is a one-line "compose from the cited
  passages" instruction, citations always carry `content`, and there is
  **no warning**. `answer_ready` is `true` with no answer model configured.
- **`local`** — the configured answer model writes answers (`composed`;
  `context_only` + warning when it can't). A BYO OpenAI-compatible endpoint
  is an advanced setting on this same path, not a third mode.

`search` / `POST /qa/search` is identical in both modes. The label/PII gate
uses the same provider-trust verdict for `ask` and `search` in both modes;
whether agent mode should treat the agent itself as an untrusted provider
is an open owner decision (handover 2026-10-21 §7.2.5). Switching modes
needs no re-ingest.

## 4. Transports

### 4.1 Local — stdio (shipped)

**Setup is in the app — nothing to build.** Run AnchorCore, open the **System
tab → MCP**, and click **Connect** for your client: it writes that app's config
(backed up first) and prints the exact command. The onboarding wizard's last step
offers the same card, and `GET /system/mcp` returns the command and args for
harness setups that have no UI.

Under the hood: the same `anchorcore` binary also runs as the stdio MCP server —
the harness spawns `anchorcore mcp`, which talks to the running app over
`127.0.0.1` (`ANCHOR_BACKEND_URL`, default `http://127.0.0.1:8000`). The app stays
the single owner of the DB, exactly like with the browser UI.

Manual configuration (equivalent to what Connect writes):

```bash
# macOS (app bundle)
claude mcp add anchorcore -- /Applications/AnchorCore.app/Contents/MacOS/AnchorCore mcp
# Windows (unzipped folder)
claude mcp add anchorcore -- C:\path\to\anchorcore.exe mcp
# opencode V2: mcp.servers entry — or `opencode mcp add anchorcore --global -- <exe> mcp`
# from a repo checkout: see "For contributors" at the end of this document
```

> Pre-R20.8 configs that point at a separate `anchorcore-mcp` file keep working if
> you copy/rename `anchorcore` to `anchorcore-mcp`, or re-run Connect (= Repair).
> Plan: [plan-mcp-zero-setup.md](./plan-mcp-zero-setup.md).

Details: stdio JSON-RPC (`initialize`/`tools/list`/`tools/call`); every `ask`/`search`
call is logged to `system_events` (`component='mcp'`, visible under
`GET /system/errors?component=mcp`).

Legacy: the B14.1 Python sidecar (`backend/anchorcore_mcp.py` +
`backend/app/mcp/`, same 6 tools) is archived history on tag
`archive/python-final`. New work is Rust
(`rust/crates/anchorcore/src/mcp.rs`).

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
| Transport (HTTP, planned) | Bearer token in `Authorization` header (MCP HTTP spec) |
| Config | `ANCHOR_MCP_TOKEN` (secret; read from env, never in DB/logs) |
| Localhost | stdio needs no token; binds 127.0.0.1 |
| Permission tiers (v2) | read-only key vs write-back key; per-team scoping |
| Audit | Every `ask`/`search` call writes a `system_events` row (`component='mcp'`: tool, query sketch, hit counts) — "who asked what" is always answerable |
| Rate limiting | Simple per-token sliding window (v2; cheap, do it when tokens exist) |

## 6. Write-back (v2, planned)

`ingest(text, source_ref, kind_hint)` — let harnesses feed decisions, docs,
and notes **back into** memory from their sessions. Harnesses become memory
producers, not just consumers. Risks to manage:

- **Trust**: ingested-from-agent items should be marked `unverified` + author
  `mcp:<token-id>` so the review UI owns confirmation — never auto-trusted.
- **Abuse**: write tokens only, rate-limited, per-team.

## 7. Where the code lives

- `rust/crates/anchorcore/src/mcp.rs` — the shipped MCP mode (R20.8): stdio
  server, the 6 tool definitions, HTTP calls into the app, audit logging.
- `rust/crates/anchorcore/src/mcp_setup.rs` — R20.9: client connect/
  disconnect (Claude Code/Desktop, Cursor, opencode, Codex).
- `backend/app/mcp/` + `backend/anchorcore_mcp.py` — legacy Python sidecar on the archive tag.
- Config: `ANCHOR_BACKEND_URL` (default `http://127.0.0.1:8000`),
  `ANCHOR_MCP_TOKEN` is optional — sent as `Bearer` to the backend when set (required for the planned HTTP transport).
- Planned: mount `/mcp` on the Axum app when the HTTP transport ships.

Deliberately unchanged by MCP: DB schema (it's an adapter, not a new data
model), retrieval internals, secrets handling (token lives in
SecretStore/env; masked in logs via existing redaction).

## 8. Client Setup Examples

**Claude Code (local stdio, works today):**
```bash
claude mcp add anchorcore -- /Applications/AnchorCore.app/Contents/MacOS/AnchorCore mcp
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

**opencode** — run `opencode mcp add anchorcore --global -- <path-to-anchorcore> mcp`,
or add `mcp.servers.anchorcore` (V2: `type: "local"`, `command` array) in
`opencode.json`; remote URL once B14.2 ships. The System tab does this for you
(R20.9).

## 9. Milestones

| # | Scope | Status |
|---|---|---|
| B14.1 | Local stdio MCP, read-only (Python sidecar + 6 tools + `/qa/search`) | ✅ DONE (v1.0.9, legacy) |
| R12.5 | Same surface as a Rust sidecar + `public_only` + audit | ✅ DONE (1.0.12, shipped) |

Open MCP work (zero-setup UX, HTTP transport, write-back, registry) is tracked in `rust/BACKLOG.md` (Phase 20 + B14.2–B14.4) — see there for current status. Plan: [plan-mcp-zero-setup.md](./plan-mcp-zero-setup.md).

## 10. Open Questions

- [x] **Decided (R20.7, 2026-10-11): local stays stdio + `runtime.json` discovery (R20.4), and streamable HTTP (B14.2) is the Team/remote answer, re-ranked ahead of the registry (B14.4).** Rationale: a local harness must own the stdio process (stdin/stdout *are* the transport), while HTTP removes the path entirely for a shared instance — one URL, bearer token, no host paths to discover. The registry is still worth doing, but as a convenience rather than as the thing that unblocks Team. Details in the plan's "Decisions recorded".
- [ ] Token scoping model for v2 (per-user vs per-team, read/write tiers)?
- [x] ~~Cap `search` k to protect Ollama/context under agent load?~~ — resolved: `k` defaults to 8, max 50, server-side.
- [ ] Should `ask` be exposed at all, or is `search` enough for agents (answers for humans)?
- [ ] Naming: expose the product as `anchorcore` (brand) or `memory` (function) in MCP tool namespaces?

---

## 11. For contributors (dev checkout)

From a repo checkout, build the same binary the release ships and point your
client at it:

```bash
cd rust && cargo build            # → rust/target/debug/anchorcore
./target/debug/anchorcore mcp     # exactly what the harness spawns
cargo test -p anchorcore          # MCP surface tests
```

`rust/crates/anchorcore/src/mcp.rs` is the implementation, `AGENTS.md` has the
full dev and release workflow, and `docs/plan-mcp-zero-setup.md` is the plan
behind §4.1 (including the `runtime.json` discovery work, R20.4).

---

*Companion docs: [product-plan.md](./product-plan.md), [architecture.md](./architecture.md), [packaging.md](./packaging.md)*
