---
name: anchorcore-setup
description: Install, launch, connect, and fully configure AnchorCore (local-first company memory) through its local REST API — first-run setup on Windows/macOS/Linux, answer-model choice (local Ollama or the user's own OpenAI-compatible AI), classification/embedding models, sources (folder, Jira, Linear, Google Drive, generic REST), source labels and cloud-trust rules, projects, PII settings, MCP agent connection, and end-to-end verification of cited answers. Use when the user asks to set up, configure, install, connect, audit, or troubleshoot AnchorCore; add their documents or trackers to it; point its answers at their own model; or asks why answers are context-only, keyword-only, or uncited.
---

# AnchorCore setup

Configure a user's AnchorCore instance end to end by driving its **local REST
API**. The running app owns the database — talk to it over HTTP, never edit
SQLite. Single DB owner is the design: app first, sidecars/clients second.

You are the operator: do the steps you can, confirm before anything destructive,
and when a step needs the user (OAuth, API keys, approvals), give the exact
command or action and wait.

## Rules (read first)

1. **Find the app before anything.** Base URL, in order: `ANCHOR_BACKEND_URL`
   env → `~/.anchorcore/runtime.json` (if present; newer builds write it) →
   `http://127.0.0.1:8000`. Probe `GET /health`; expected `{"status":"ok"}`.
   Anything else means the app is not up — see step 1.
2. **CSRF:** every `POST`/`PUT`/`PATCH`/`DELETE` needs `X-CSRF-Token`. Fetch one
   with `GET /csrf` and reuse it for the session. A `403 … CSRF` means you forgot.
3. **Auth:** local installs need no login. If `GET /auth/status` returns
   `{"enabled":true}` or you get `401`, `POST /auth/login {"email","password"}` →
   `access_token`, then send `Authorization: Bearer <token>` (CSRF does not
   apply when auth is on).
4. **Secrets** (API keys, tokens, passwords) go to the OS keychain, are masked
   as `***set***` on read, and are never logged. To change one, send the new
   value; to keep one, send `***set***` or omit the field; an explicit `""`
   clears it. Never echo secrets into chat, logs, or shell history.
5. **Never:** edit the SQLite file, set `ANCHOR_CSRF_DISABLE=1`, delete the data
   dir to "fix" things, or send `sensitive`/`pii` content to an untrusted cloud
   provider — the app gates that for a reason.

## 1. Install / first run (only if `/health` fails)

Prebuilt artifacts — users never need Rust or Node:
`https://github.com/Atsirkunov/anchorcore/releases/latest/download/AnchorCore-{windows,macos,linux}.zip`
Unzip: Windows/Linux `anchorcore.exe` / `anchorcore`; macOS `AnchorCore.app`
(right-click → Open once — not notarized — or `xattr -dr com.apple.quarantine AnchorCore.app`).

Launch detached so it survives your shell:
- Windows (PowerShell): `Start-Process -FilePath 'C:\path\anchorcore.exe'`
- macOS: `open -a '/Applications/AnchorCore.app'` (or run `AnchorCore.app/Contents/MacOS/AnchorCore`)
- Linux: `nohup ./anchorcore >/dev/null 2>&1 &`

Terminal launch flags: `--port 8123`, `--data-dir <path>`, and
`ANCHOR_OPEN_BROWSER=0` to suppress the browser tab. Double-click defaults:
port 8000, data dir `~/.anchorcore`, UI at `http://127.0.0.1:8000`.

Poll `/health` until `ok` (allow ~30 s on first run for migrations). Then read
before you write:

| Call | What it tells you |
|---|---|
| `GET /system/status` | version, models configured/missing, `answer_ready`, pending embeddings, failing sources |
| `GET /system/onboarding` | `needs_wizard`, `sources_count`, sample availability |
| `GET /system/mcp` | the exact command MCP clients must run |
| `GET /system/errors?limit=20` | recent failures (empty on a clean box) |

Summarize the state in plain language before changing anything.

## 2. Decide how answers are produced (do this early)

Three modes — pick with the user, configure, then **verify**:

1. **Agent-native (recommended when the user chats through Claude Code, Codex,
   opencode, Cursor, …):** the agent calls `search` (MCP) or
   `POST /qa/search` and answers with *its own* model. AnchorCore needs no
   answer model at all. This is the bring-your-own-model path.
2. **Local model (Ollama):** `POST /system/ollama/start`, then
   `POST /system/ollama/pull {"models":["llama3.2:3b","nomic-embed-text"]}`,
   poll `GET /system/ollama/pulls`. Leaving `classifier_base_url` /
   `embed_base_url` empty makes those roles fall back to `ollama_base_url`
   (`http://localhost:11434`). For **answers**, set `answer_base_url` to
   `http://localhost:11434/v1` and `answer_model` to a pulled chat model
   (e.g. `llama3.2:3b`) explicitly — the answer provider has no Ollama fallback
   of its own and quietly returns `context_only` otherwise.
3. **The user's own AI (any OpenAI-compatible endpoint):** `PUT /settings` with
   `{"answer_base_url":"https://…/v1","answer_model":"…","answer_api_key":"…"}`
   (classifier/embed accept the same `*_base_url` / `*_model` / `*_api_key`
   keys), then `POST /settings/test-connection {"provider":"answer"}`.

**Honesty contract — never skip this.** `POST /qa` (and MCP `ask`) reports how
the answer was produced:

- `mode: "composed"` — a model wrote it.
- `mode: "context_only"` — **no model ran** (missing key, unreachable endpoint,
  missing local model); the text is extractive context, not an answer.
- `mode: "none"` / `refusal: "gated_provider"` — nothing was returned.
- `retrieval: "hybrid"` is full search; `"keyword_only"` means embeddings were
  unavailable (no `embed_model`), so ranking is keyword-only.

After configuring, run one real question and check both flags. If the user
expects model-written answers and you get `context_only`, **say so explicitly,
name the cause** (`/system/status` → `answer.provider` is `ollama|configured|missing`;
`GET /system/errors?component=qa`), fix it, and re-probe. Never present
`context_only` output as a model answer — and never let a harness degrade
silently to a model the user did not choose.

Cloud providers are trusted for `sensitive`/`pii` sources and PII-flagged chunks
only when the host is in `ANCHOR_TRUSTED_PROVIDERS` or `ANCHOR_CLOUD_TRUST=1`
(both env vars — restart needed); localhost is always trusted. If the user is
not comfortable with that, use a local model.

## 3. Index the knowledge (sources)

`GET /sources` first. Create with `POST /sources`:

```json
{"connector":"folder","name":"Handbook","label":"internal","config":{"path":"/abs/path"}}
```

| Connector | `config` fields |
|---|---|
| `folder` | `path` (absolute). A watcher auto-syncs edits. |
| `jira` | `base_url`, `email`, `token`, `project` (keys, comma-separated). The project-picker endpoint is a stub in current builds — ask the user for keys. |
| `linear` | `api_key`, `team` (`ENG` or `ENG,PM`) |
| `gdrive` | `folder_id`, `token` (OAuth bearer) |
| `rest` | `base_url`, `list_path`, `method` (`GET`/`POST`), `auth_mode` (`none`/`bearer`/`basic`/`header`) plus `token` / `email`+`password` / `header_name`+`api_key`; `items_path`; `map_id`, `map_title`, `map_text` (comma-separated pointers), `map_author`, `map_updated`; pagination `page_mode` (`none`/`token`/`page`/`offset`) + params; `since_param`; `ref_prefix` |

For `rest`, always preview first:
`POST /sources/rest/preview {"config":{…}}` fetches one page and shows the
mapped docs without saving. Fix the mapping, then create.

**Labels decide trust — ask, don't guess:** `internal` (default), `public`
(only these serve `/qa/public` and `public_only` MCP calls),
`sensitive`, `pii` (never sent to untrusted cloud providers). Change later with
`PUT /sources/{id} {"label":"…"}`; rename/enable via the same endpoint.

Then sync: `POST /sources/{id}/sync` → poll `GET /sources/jobs/{id}` until
`done` or `failed` (two jobs run at once; one job per source). `POST
/sources/{id}/reclassify` re-runs understanding. For a first demo, index the
repo's `sample/` corpus (release zips do not include it; clone
`--depth 1 --filter=blob:none --sparse`, `sparse-checkout set sample`) and ask
"what was decided about security transfers, and why?".

## 4. Projects, PII, review (as needed)

- Projects scope Ask: `POST /projects {"name":"Payments","source_ids":[1,2]}`;
  `PATCH /projects/{id} {"source_ids":[…],"is_default":true}`.
- PII: `GET /pii/config` → `PUT /pii/config {"custom_words":["…"],
  "disabled_categories":["…"]}`; review `GET /pii/review?only_flagged=true`;
  decide `POST /pii/review/{chunk_id} {"is_pii":true|false}`; scan a source
  `POST /pii/scan/{source_id}`.
- Housekeeping: `GET /review/low-confidence`, `GET /review/duplicates`,
  `POST /review/merge {"proposal_id":…,"decision":"merge"|"dismiss"}`.

## 5. Connect the user's agent (MCP)

`GET /system/mcp` returns the canonical command (`command`+`args:["mcp"]` on
current builds). Builds up to 1.0.14 have no such endpoint: the MCP helper sits
next to the app binary (`anchorcore-mcp.exe` / `anchorcore-mcp`;
`AnchorCore.app/Contents/MacOS/anchorcore-mcp`) and takes no `mcp` argument. The
app must be running — the MCP process talks to it over `127.0.0.1` (single DB
owner). When the port isn't 8000, older builds need
`ANCHOR_BACKEND_URL=http://127.0.0.1:<port>` in the client entry.

Newer builds expose one-click setup — prefer it when present:

```
GET  /system/mcp/clients                               → detection + connected state
POST /system/mcp/connect    {"client":"claude-code"}   # ids: claude-code,
POST /system/mcp/disconnect {"client":"…"}             # claude-desktop, cursor, opencode, codex
```

Writes are explicit and backup+merge (never clobber an existing config) — ask
the user first. If those endpoints 404, print the manual line instead:
- Claude Code: `claude mcp add anchorcore -- "<path>" mcp` (≤1.0.14: the
  `anchorcore-mcp` helper path, no `mcp` argument)
- Claude Desktop / Cursor JSON: `{"mcpServers":{"anchorcore":{"command":"<path>","args":["mcp"]}}}`
- opencode: `mcp.servers.anchorcore = {"type":"local","command":["<path>","mcp"]}`
- Codex (TOML): `[mcp_servers.anchorcore]`, `command = "<path>"`, `args = ["mcp"]`

Verify after a harness restart: `tools/list` shows the 6 read-only tools
(`ask`, `search`, `get_entity`, `get_source`, `list_sources`, `memory_status`),
and `search "…"` returns cited chunks. Calls are audited (rows under
`/system/errors?component=mcp`; `GET /system/mcp` reports the counter on builds
that have it).

## 6. Verify end to end (always)

1. `POST /qa/search {"query":"…","k":8}` → hits with `source_ref` and scores;
   `retrieval` matches expectations.
2. `POST /qa {"question":"…"}` → `mode` per the honesty contract, ≥1 citation.
3. `GET /system/status` → `answer_ready: true` (or an agreed degraded state),
   `pending_embeddings` trending to 0, no failing sources.
4. `GET /system/errors?level=warning&limit=20` → every recent warning explained.

Report back: what you configured, what you verified, what is degraded and why.

## Troubleshooting (symptom → cause → fix)

| Symptom | Cause | Fix |
|---|---|---|
| connection refused / "backend unreachable" | app not running | start it (step 1), check `/health` |
| `403` "missing or invalid CSRF token" | write without `X-CSRF-Token` | `GET /csrf`, retry with the header |
| `401` | auth enabled | `POST /auth/login`, use `Authorization: Bearer` |
| `retrieval: keyword_only` | embed model/base unreachable or unset | pull `nomic-embed-text` or set `embed_base_url`, then sync/reclassify to re-embed |
| `mode: context_only` | answer provider missing or broken | configure step 2, `test-connection`, re-probe |
| `refusal: gated_provider` | sensitive/PII content + untrusted cloud | local model, `ANCHOR_TRUSTED_PROVIDERS`, or user-approved `ANCHOR_CLOUD_TRUST=1` |
| job `failed` | connector/auth error | `GET /sources/jobs/{id}`, `/system/errors`, fix config, re-sync |
| settings seem ignored | empty string means "unset" (falls back to env/default) | set real values; confirm via `/system/status` |

**PowerShell notes:** use `curl.exe` (not the `curl` alias) for real curl, or
`Invoke-RestMethod -Headers @{"X-CSRF-Token"=$csrf}`; prefer `ConvertTo-Json`
over hand-written JSON; `$env:TEMP` replaces `/tmp`. Logs live in
`<data-dir>/anchorcore.log` (`GET /system/logs`). Never build from source for a
user — the shipped binary already embeds the UI.
