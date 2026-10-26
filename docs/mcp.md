# AnchorCore — Agent Connectivity via MCP

> Goal: let any AI harness (Claude Code, Codex, opencode, Cursor, …) connect
> to AnchorCore's memory — locally or against a shared instance — and
> consume it with full provenance.
> Status: **shipped** — `anchorcore mcp` (same binary, stdio, 9 read-only
> tools, `public_only` gate, ask/search audit). Every tool has a REST
> equivalent (the `anchorcore-search` skill), so no feature is MCP-only. HTTP
> transport and write-back are planned (B14.2/B14.3).

---

## 1. Why MCP

Harnesses (Claude Code, Codex, opencode, Cursor…) start every session with no
memory and no provenance. They all speak MCP, so one adapter serves every
harness; each tool result carries `source_ref`, status and provenance from the
same retrieval the UI uses. Agents that prefer plain HTTP use the REST skill.

## 2. Tool surface (v1 — read-only)

| Tool | Parameters | Returns | Maps to |
|---|---|---|---|
| `ask` | `question` (required), `project_id?`, `public_only?`, `include_content?` | composed answer (local mode) or `agent_compose` evidence (agent mode, §2.2) + citations with provenance (item id + file path + title + env class on every citation; full chunk bytes on opt-in — always in agent mode — capped at 4000 chars with `content_truncated`) + `mode`/`warning` flags (§2.1) | `POST /qa`, `POST /qa/public` |
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

### 2.1 Bring your own model (BYOM) — the answer contract (R23.4)

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
  `warning` (`agent_compose` is by design, §2.2, and never warns). Agents must not quote
  warned text as an answer: use the citations, or fall back to `search` +
  their own model. The Ask tab renders the same warning as an amber
  banner, so humans see it too.
- **No Ollama fallback for answers.** `answer_base_url` defaults to
  `https://api.openai.com/v1` — unlike embeddings, it never falls
  back to Ollama. A local-only setup must set
  `http://localhost:11434/v1` explicitly, or answers stay `context_only`
  with the warning shown. The Settings "Run compose probe" button (and
  the wizard's step-3 question) verifies the endpoint end to end: only
  `mode: "composed"` means answers really work.

### 2.2 Agentic vs local LLM — `llm_mode` (R45.2)

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

## 3. Transports

### 3.1 Local — stdio (shipped)

**Setup is in the app — nothing to build.** Run AnchorCore, open the **System
tab → MCP**, and click **Connect** for your client: it writes that app's config
(backed up first) and prints the exact command. The onboarding wizard's last step
offers the same card, and `GET /system/mcp` returns the command and args for
harness setups that have no UI.

Under the hood: the same `anchorcore` binary also runs as the stdio MCP server —
the harness spawns `anchorcore mcp`, which talks to the running app over
`127.0.0.1`. The app stays the single owner of the DB, exactly like with the
browser UI.

**Finding the app (R20.4) — no env vars on any port.** After its listener binds,
the app writes `runtime.json` (`port`, `host`, `pid`, `version`; atomic temp +
rename) into its data dir and removes it on graceful shutdown. `anchorcore mcp`
resolves the backend in this order:

1. `ANCHOR_BACKEND_URL`, when set — always wins.
2. `runtime.json` in the data dir, only if that server answers `GET /health`
   (a file left by a crash is ignored).
3. `http://127.0.0.1:8000`.

The MCP side resolves the data dir like the app does: `anchorcore mcp --data-dir
<dir>` → `ANCHOR_DATA_DIR` → the `ANCHOR_DATABASE_URL` parent → `~/.anchorcore`. So
`anchorcore --port 8123` needs nothing extra. When the app runs on any other data
dir (`--data-dir`, `ANCHOR_DATA_DIR`), the System card and Connect add
`--data-dir <absolute dir>` to the MCP entry for you. The discovered URL is cached until a call fails
to connect, then re-resolved, so restarting the app on another port needs no
client restart. When nothing answers, tool errors say how to start the app.

Manual configuration (equivalent to what Connect writes):

```bash
# macOS (app bundle)
claude mcp add anchorcore -- /Applications/AnchorCore.app/Contents/MacOS/AnchorCore mcp
# Windows (unzipped folder)
claude mcp add anchorcore -- C:\path\to\anchorcore.exe mcp
# opencode V2: mcp.servers entry — or `opencode mcp add anchorcore --global -- <exe> mcp`
# from a repo checkout: see "For contributors" at the end of this document
```

> Old configs pointing at a separate `anchorcore-mcp` file: re-run Connect (= Repair).

Details: stdio JSON-RPC (`initialize`/`tools/list`/`tools/call`); every `ask`/`search`
call is logged to `system_events` (`component='mcp'`, visible under
`GET /system/errors?component=mcp`).

### 3.2 Centralized — streamable HTTP (planned, B14.2)

MCP **streamable HTTP transport mounted on the same Axum app**, e.g.
`POST https://your-host/mcp`:

```bash
claude mcp add anchorcore --transport http https://your-host/mcp
```

- One process serves UI + REST API + MCP — no extra runtime.
- Bearer-token auth (see §4) — opt-in; disabled on localhost by default.
- This is the hook for the v2 team story: shared memory that every agent in
  the org reads from.

## 4. Auth & Auditing

| Concern | Approach |
|---|---|
| Transport (HTTP, planned) | Bearer token in `Authorization` header (MCP HTTP spec) |
| Config | `ANCHOR_MCP_TOKEN` (secret; read from env, never in DB/logs) |
| Localhost | stdio needs no token; binds 127.0.0.1 |
| Permission tiers (v2) | read-only key vs write-back key; per-team scoping |
| Audit | Every `ask`/`search` call writes a `system_events` row (`component='mcp'`: tool, query sketch, hit counts) — "who asked what" is always answerable |
| Rate limiting | Simple per-token sliding window (v2; cheap, do it when tokens exist) |

## 5. Write-back (v2, planned)

`ingest(text, source_ref, kind_hint)` — let harnesses feed decisions, docs,
and notes **back into** memory from their sessions. Harnesses become memory
producers, not just consumers. Risks to manage:

- **Trust**: ingested-from-agent items should be marked `unverified` + author
  `mcp:<token-id>` so the review UI owns confirmation — never auto-trusted.
- **Abuse**: write tokens only, rate-limited, per-team.

## 6. Where the code lives

- `rust/crates/anchorcore/src/mcp.rs` — stdio server, the tool definitions,
  HTTP calls into the app, audit logging.
- `rust/crates/anchorcore/src/mcp_setup.rs` — R20.9: client connect/
  disconnect (Claude Code/Desktop, Cursor, opencode, Codex).
- `rust/crates/anchorcore/src/runtime.rs` — R20.4: `runtime.json` write/read
  and the shared data-dir resolution.
- Config: `ANCHOR_BACKEND_URL` (overrides `runtime.json` discovery; default
  `http://127.0.0.1:8000`),
  `ANCHOR_MCP_TOKEN` is optional — sent as `Bearer` to the backend when set (required for the planned HTTP transport).
- Planned: mount `/mcp` on the Axum app when the HTTP transport ships.

Deliberately unchanged by MCP: DB schema (it's an adapter, not a new data
model), retrieval internals, secrets handling (token lives in
SecretStore/env; masked in logs via existing redaction).

## 7. Open questions

- [x] **Decided (R20.7, 2026-10-11): local stays stdio + `runtime.json` discovery (R20.4), and streamable HTTP (B14.2) is the Team/remote answer, re-ranked ahead of the registry (B14.4).** Rationale: a local harness must own the stdio process (stdin/stdout *are* the transport), while HTTP removes the path entirely for a shared instance — one URL, bearer token, no host paths to discover. The registry is still worth doing, but as a convenience rather than as the thing that unblocks Team. Details in the plan's "Decisions recorded".
- [ ] Token scoping model for v2 (per-user vs per-team, read/write tiers)?
- [x] ~~Cap `search` k to protect Ollama/context under agent load?~~ — resolved: `k` defaults to 8, max 50, server-side.
- [ ] Naming: expose the product as `anchorcore` (brand) or `memory` (function) in MCP tool namespaces?

---

## 8. For contributors (dev checkout)

From a repo checkout, build the same binary the release ships and point your
client at it:

```bash
cd rust && cargo build            # → rust/target/debug/anchorcore
./target/debug/anchorcore mcp     # exactly what the harness spawns
cargo test -p anchorcore          # MCP surface tests
```

`rust/crates/anchorcore/src/mcp.rs` is the implementation and `AGENTS.md` has the
full dev and release workflow. Open MCP work (CI search smoke R20.10, HTTP
transport B14.2) is in `rust/BACKLOG.md`.

---

*Companion docs: [product-plan.md](./product-plan.md), [architecture.md](./architecture.md), [packaging.md](./packaging.md)*
