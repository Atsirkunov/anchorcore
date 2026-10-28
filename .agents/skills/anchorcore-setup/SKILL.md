---
name: anchorcore-setup
description: Install, update, launch, connect, and fully configure AnchorCore (local-first company memory) through its local REST API — always check the installed build and any installer copy on the PC against the newest GitHub release and update when stale, first-run setup on Windows/macOS/Linux, who writes answers (the user's agent, local Ollama, or the user's own OpenAI-compatible AI), embedding models, sources (folder, Jira, Linear, Google Drive, generic REST), source labels and cloud-trust rules, projects, per-document Sensitive marking, MCP agent connection (REST-API fallback when MCP can't be installed), and end-to-end verification of cited answers. Use when the user asks to set up, configure, install, update, upgrade, connect, audit, or troubleshoot AnchorCore; add their documents or trackers to it; point its answers at their own model; or asks why answers are context-only, keyword-only, or uncited.
---

# AnchorCore setup

Configure a user's AnchorCore instance end to end by driving its **local REST
API**. The running app owns the database — talk to it over HTTP, never edit
SQLite. Single DB owner is the design: app first, sidecars/clients second.

You are the operator: do the steps you can, confirm before anything destructive,
and when a step needs the user (OAuth, API keys, approvals), give the exact
command or action and wait.

## Rules (read first)

1. **Find the app before anything.** Base URL: `ANCHOR_BACKEND_URL` env if
   set, else the `port` in `~/.anchorcore/runtime.json` (the running app writes
   it; use `http://127.0.0.1:<port>`), else `http://127.0.0.1:8000`. Probe
   `GET /health`; expected `{"status":"ok"}`.
   Anything else means the app is not up — see step 1. Exception: when the
   harness reports a sandbox, permission or network-policy denial (not a
   refused connection), the app may be fine and your sandbox is blocking
   localhost. Ask once for the narrowest access ("allow 127.0.0.1:<port>, or
   run this one request outside the sandbox?"), never bypass the sandbox
   unasked, and on a no give the user the command to run themselves
   (`! curl -s http://127.0.0.1:<port>/health` in Claude Code).
2. **Newest version first.** Before configuring anything, check the installed
   build (and any installer copy you are about to use) against the newest GitHub
   release — step 1. If it is older, update it first. Never set up, launch, or
   unzip a stale build lying around on the PC.
3. **CSRF:** every `POST`/`PUT`/`PATCH`/`DELETE` needs `X-CSRF-Token`. Fetch one
   with `GET /csrf` and reuse it for the session. A `403 … CSRF` means you forgot.
4. **Auth:** local installs need no login. If `GET /auth/status` returns
   `{"enabled":true}` or you get `401`, `POST /auth/login {"email","password"}` →
   `access_token`, then send `Authorization: Bearer <token>` (CSRF does not
   apply when auth is on).
5. **Secrets** (API keys, tokens, passwords) go to the OS keychain, are masked
   as `***set***` on read, and are never logged. To change one, send the new
   value; to keep one, send `***set***` or omit the field; an explicit `""`
   clears it. Never echo secrets into chat, logs, or shell history.
6. **Never:** edit the SQLite file, set `ANCHOR_CSRF_DISABLE=1`, delete the data
   dir to "fix" things, or send `sensitive` content to an untrusted cloud
   provider — the app gates that for a reason.

## 1. Install / update — newest release first

Run this check **every session** — also when `/health` is fine: a stale install
must never be configured, and a zip lying around (Downloads, Desktop) must never
be trusted over `releases/latest`.

Installed version:
- app running → `GET /system/status` → `version`
- app on disk, not running → `<binary> --version` when the build supports it; on
  builds that reject the flag (`unexpected argument '--version'`), boot that copy
  once on a throwaway data dir (`--data-dir <temp dir>`, free `--port`,
  `ANCHOR_OPEN_BROWSER=0`), read `/system/status`, then stop it
- a downloaded zip → unverifiable until it runs: don't trust it, download fresh

Newest release: `https://api.github.com/repos/Atsirkunov/anchorcore/releases/latest`
→ `tag_name` (strip the leading `v`) — or, on a running app that has it,
`GET /system/update`. PowerShell:
`(Invoke-RestMethod https://api.github.com/repos/Atsirkunov/anchorcore/releases/latest).tag_name`;
no JSON tooling: the `location:` header of
`curl -sI https://github.com/Atsirkunov/anchorcore/releases/latest`. Compare
numerically — `1.0.9` is older than `1.0.20`; string compare gets that wrong.
Only a strictly older install is replaced, so a source/dev build ahead of the
release is left alone. No network → say so and continue with what is installed
(note any version you could not verify).

| Installed | Do |
|---|---|
| same as newest, or newer (source/dev build) | use it as is |
| older | download the newest zip, replace in place, relaunch |
| missing, or only an unverifiable zip | download the newest zip, fresh install |
| a `target/`/cargo source build that is older | don't overwrite it; report the newer release and let the user decide |

Prebuilt artifacts — users never need Rust or Node:
`https://github.com/Atsirkunov/anchorcore/releases/latest/download/AnchorCore-{windows,macos,linux}.zip`
Unzip: Windows/Linux `anchorcore.exe` / `anchorcore`; macOS `AnchorCore.app`
(not notarized: first launch needs System Settings → Privacy & Security → Open
Anyway, or `xattr -dr com.apple.quarantine AnchorCore.app`).
Keep the app in one place so the next check is cheap: Windows
`%LOCALAPPDATA%\AnchorCore\`, macOS `/Applications/AnchorCore.app` (or `~/Applications`), Linux
`~/.local/share/anchorcore\` — or wherever the user already keeps it.

**Update in place** (same folder, same data dir):
1. Stop the app — Windows locks the running `.exe` (`Stop-Process -Name anchorcore`),
   macOS quit the app, Linux kill the pid.
2. Unzip the newest archive over the same folder — the path the app runs from
   (Windows `(Get-Process anchorcore).Path`; ask the user if it is unknown).
   macOS: replace the whole `AnchorCore.app` bundle, don't merge into it.
3. Relaunch (below) and confirm `GET /system/status.version` equals the newest.
An update replaces the binary only — user data (`~/.anchorcore`) is untouched,
migrations run on the next start. Report what changed: `old → new`.

Launch detached so it survives your shell:
- Windows (PowerShell): `Start-Process -FilePath 'C:\path\anchorcore.exe'`
- macOS: `open /Applications/AnchorCore.app` (or wherever it lives; or run `AnchorCore.app/Contents/MacOS/AnchorCore`)
- Linux: `nohup ./anchorcore >/dev/null 2>&1 &`

Terminal launch flags: `--port 8123`, `--data-dir <path>`, and
`ANCHOR_OPEN_BROWSER=0` to suppress the browser tab. Double-click defaults:
port 8000, data dir `~/.anchorcore`, UI at `http://127.0.0.1:8000`.

Poll `/health` until `ok` (allow ~30 s on first run for migrations). Then read
before you write:

| Call | What it tells you |
|---|---|
| `GET /system/status` | version, `llm_mode` (`agent`/`local`), models configured/missing, `answer_ready`, pending embeddings, failing sources |
| `GET /system/onboarding` | `needs_wizard`, `sources_count`, sample availability |
| `GET /system/mcp` | the exact command MCP clients must run |
| `GET /system/errors?limit=20` | recent failures (empty on a clean box) |

Summarize the state in plain language before changing anything.

## 2. Decide how answers are produced (do this early)

Two modes (`llm_mode`), the second with two providers — pick with the user,
configure, then **verify**. Indexing never needs a chat model either way.

1. **Agent mode (recommended when the user chats through Claude Code, Codex,
   opencode, Cursor, …):** `PUT /settings {"llm_mode":"agent"}`. The agent
   calls `search` (MCP) or `POST /qa/search` (REST skill) and answers with
   *its own* model; `ask` / `POST /qa` return `mode: "agent_compose"`
   (gated citations with content, no server model, no warning). AnchorCore
   needs no answer model at all and `answer_ready` reads `true`. This is the
   bring-your-own-model path. (`llm_mode` defaults to `local`.)
2. **Local mode, Ollama** (`llm_mode` `local`, the default): `POST /system/ollama/start`, then
   `POST /system/ollama/pull {"models":["llama3.2:3b","nomic-embed-text"]}`,
   poll `GET /system/ollama/pulls`. Leaving `embed_base_url` empty makes
   embeddings fall back to `ollama_base_url`
   (`http://localhost:11434`). For **answers**, set `answer_base_url` to
   `http://localhost:11434/v1` and `answer_model` to a pulled chat model
   (e.g. `llama3.2:3b`) explicitly — the answer provider has no Ollama fallback
   of its own and quietly returns `context_only` otherwise.
3. **Local mode, the user's own AI (any OpenAI-compatible endpoint):** `PUT /settings` with
   `{"answer_base_url":"https://…/v1","answer_model":"…","answer_api_key":"…"}`
   (embeddings accept the same `embed_base_url` / `embed_model` /
   `embed_api_key` keys), then `POST /settings/test-connection {"provider":"answer"}`.

**Embeddings (search vectors) — independent of who writes answers.** The
simplest local path needs no Ollama: `GET /system/embed-model` shows
`ready`/`active`; `POST /system/embed-model/download` fetches the built-in
nomic-embed-text v1.5 (≈550 MB, pinned + checksummed; poll the same GET for
`progress`). `embed_provider` is `auto` (default: Ollama or a configured
cloud endpoint when it serves — faster bulk indexing — else the built-in
model once downloaded), `builtin`, or `ollama`. Built-in and Ollama
`nomic-embed-text` vectors are one family — switching re-embeds nothing.
Headless: `anchorcore embed-model fetch` (add `--offline` to only verify
files already dropped into the model directory).

**Honesty contract — never skip this.** `POST /qa` (and MCP `ask`) reports how
the answer was produced:

- `mode: "composed"` — a model wrote it.
- `mode: "agent_compose"` — agent mode (`llm_mode: agent`): by design no
  server model ran; compose the answer yourself from the citations. Not a
  degradation, no warning.
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

Cloud providers are trusted for `sensitive` sources and Sensitive-marked
documents only when the host is in `ANCHOR_TRUSTED_PROVIDERS` or `ANCHOR_CLOUD_TRUST=1`
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
`sensitive` (never sent to untrusted cloud providers). Change later with
`PUT /sources/{id} {"label":"…"}`; rename/enable via the same endpoint.

Then sync: `POST /sources/{id}/sync` → poll `GET /sources/jobs/{id}` until
`done` or `failed` (two jobs run at once; one job per source). `POST
/sources/{id}/reclassify` re-runs understanding. For a first demo, index the
repo's `sample/` corpus (release zips do not include it; clone
`--depth 1 --filter=blob:none --sparse`, `sparse-checkout set sample`) and ask
"what was decided about instant payments, and why?".

## 4. Projects, Sensitive marks, review (as needed)

- Projects scope Ask: `POST /projects {"name":"Payments","source_ids":[1,2]}`;
  `PATCH /projects/{id} {"source_ids":[…],"is_default":true}`.
- Sensitive marks (people-driven, escalate-only): `PATCH /items/{id} {"sensitive_override":true}`
  marks a whole document, `null` clears it back to inheriting the source label
  (`false` is reserved); entities carry their parent item's effective state as `item_sensitive`.
  A marked document is treated like a `sensitive` source (R35.5 caller rule):
  its chunks are never sent to an untrusted cloud embedder, and ask, search,
  `/decisions`, `/actions`, `/media` and the read surfaces (`GET /items/:id`,
  node text, section/tag chunks, entity reads) withhold it from every caller
  but the app UI (`blocked`, `refusal: gated_provider` when nothing survives;
  single-item reads answer 403). `public_only` / `/qa/public` drop it whatever
  the provider. Mark before connecting a cloud embedder — vectors already sent
  are not recalled. The auto-detect/review endpoints are retired — do not call
  `/pii/*`.
- Housekeeping: `GET /review/low-confidence`, `GET /review/duplicates`,
  `POST /review/merge {"proposal_id":…,"decision":"merge"|"dismiss"}`.

## 5. Connect the user's agent (MCP preferred, API fallback)

End this step with exactly one working connection: MCP when it goes in, the
API fallback below when it doesn't. MCP first:

`GET /system/mcp` returns the canonical command (`command` + `args:["mcp"]`).
The app must be running — the MCP process talks to it over `127.0.0.1`
(single DB owner). Any port works with no env vars: `anchorcore mcp` reads the
port from `runtime.json` in the data dir. Only a custom `--data-dir` needs the
same `--data-dir <dir>` appended to the MCP args (or `ANCHOR_DATA_DIR`);
`ANCHOR_BACKEND_URL` still overrides everything.

One-click setup:

```
GET  /system/mcp/clients                               → detection + connected state
POST /system/mcp/connect    {"client":"claude-code"}   # ids: claude-code,
POST /system/mcp/disconnect {"client":"…"}             # claude-desktop, cursor, opencode, codex
```

Writes are explicit and backup+merge (never clobber an existing config) — ask
the user first. Manual equivalents (for harnesses the endpoint doesn't know):
- Claude Code: `claude mcp add anchorcore -- "<path>" mcp`
- Claude Desktop / Cursor JSON: `{"mcpServers":{"anchorcore":{"command":"<path>","args":["mcp"]}}}`
- opencode: `mcp.servers.anchorcore = {"type":"local","command":["<path>","mcp"]}`
- Codex (TOML): `[mcp_servers.anchorcore]`, `command = "<path>"`, `args = ["mcp"]`

Verify after a harness restart: `tools/list` shows the 9 read-only tools
(`ask`, `search`, `decisions`, `actions`, `get_asset`, `get_entity`,
`get_source`, `list_sources`, `memory_status`), and `search "…"` returns cited
chunks. Calls are audited (rows under `/system/errors?component=mcp`;
`GET /system/mcp` reports the counter).

### MCP can't be installed, blocked, or unreachable — fall back to the API skill

Prefer MCP above — it is the live connection. When MCP can't go in, fall back
to the plain REST API instead of leaving the agent unconnected:

- MCP can't be installed (the harness has no MCP support, or no way to add
  a server entry).
- MCP is blocked (the harness can't spawn the local `anchorcore mcp`
  process, or `tools/list` still shows no anchorcore tools after a restart).
- MCP is not reachable (its tools error or time out) while the app itself
  answers `GET /health`.

The fallback is the read-only `ask-docs` skill: ranked search,
whole-file fetch, line windows (`GET /items/:id?lines=a-b`), and section
outlines and text (`GET /nodes/:id?include=parent,children,siblings,text`) over the same local REST API (same base
URL, CSRF, and auth rules as this skill) — no MCP process needed. It never
writes, resyncs, or reclassifies. **Either/or, never both:** with MCP tools
*and* the search skill active, agents duplicate effort (the same query goes
out over MCP and over plain HTTP). If you install the skill, do not also
connect MCP — and if MCP later starts working, remove the skill again.

Before installing, ask once: "It installs as `/ask-docs`. Do you want to use a
different handle for it?" The handle is what they type after `/` and the
skill's folder name: 1–64 lowercase letters, digits or inner hyphens. If the
client's row in `GET /system/skills/install-targets` lists `search_handles`,
offer the one already there as the default.

Install it through the app (the binary embeds both skills):
`GET /system/skills/install-targets` lists clients, then
`POST /system/skills/search/install {"client":"claude-code","handle":"ask-docs"}`
writes it into that client's skills dir (backup first; same client ids as MCP
connect; `handle` is optional). Any older copy under another handle (including
the old `anchorcore-search`) is retired, listed in `retired`, so only one copy
triggers; a different skill already using that handle is refused, never
overwritten. For a harness the app doesn't know, save `GET
/system/skills/search` (raw markdown) to `<skills dir>/<handle>/SKILL.md` and
set its `name:` line to the handle.

Restart the harness (skills load at start), then verify: ask it to search
AnchorCore for a known term — expect ranked hits with citations — and confirm
`tools/list` shows **no** anchorcore tools (the either/or check). Connection
refused means the app itself is not up — start it per step 1.

## 6. Verify end to end (always)

1. `POST /qa/search {"query":"…","k":8}` → hits with `source_ref` and scores;
   `retrieval` matches expectations.
2. `POST /qa {"question":"…"}` → `mode` per the honesty contract (`agent_compose`
   in agent mode, `composed` in local mode), ≥1 citation.
3. `GET /system/status` → `version` equals the newest release (step 1),
   `answer_ready: true` (or an agreed degraded state), `pending_embeddings`
   trending to 0, no failing sources.
4. `GET /system/errors?level=warning&limit=20` → every recent warning explained.

Report back: what you configured, what you verified, what is degraded and why.

## Troubleshooting (symptom → cause → fix)

| Symptom | Cause | Fix |
|---|---|---|
| connection refused / "backend unreachable" | app not running | start it (step 1), check `/health` |
| `version` older than `releases/latest` | stale build in use | update in place (step 1): stop → replace → relaunch, then re-probe |
| local zip/binary version can't be verified | untrusted copy on the PC | download the newest release; don't unzip the local one |
| `403` "missing or invalid CSRF token" | write without `X-CSRF-Token` | `GET /csrf`, retry with the header |
| `401` | auth enabled | `POST /auth/login`, use `Authorization: Bearer` |
| `retrieval: keyword_only` | embed model/base unreachable or unset | pull `nomic-embed-text` or set `embed_base_url`, then sync/reclassify to re-embed |
| `mode: context_only` | answer provider missing or broken | configure step 2, `test-connection`, re-probe |
| `refusal: gated_provider` | Sensitive-labelled or Sensitive-marked content + untrusted cloud | local model, `ANCHOR_TRUSTED_PROVIDERS`, or user-approved `ANCHOR_CLOUD_TRUST=1` |
| job `failed` | connector/auth error | `GET /sources/jobs/{id}`, `/system/errors`, fix config, re-sync |
| settings seem ignored | empty string means "unset" (falls back to env/default) | set real values; confirm via `/system/status` |

**PowerShell notes:** use `curl.exe` (not the `curl` alias) for real curl, or
`Invoke-RestMethod -Headers @{"X-CSRF-Token"=$csrf}`; prefer `ConvertTo-Json`
over hand-written JSON; `$env:TEMP` replaces `/tmp`. Logs live in
`<data-dir>/anchorcore.log` (`GET /system/logs`). Never build from source for a
user — the shipped binary already embeds the UI.
