# AnchorCore

Connect your knowledge to any AI model. An AI memory layer / knowledge operating system for teams.

## Docs

- [Product Plan](./docs/product-plan.md) — what we're building, for whom, and why (+ full backlog)
- [Architecture](./docs/architecture.md) — system view + architecture diagram
- [Packaging](./docs/packaging.md) — Windows exe (done), macOS plan
- [Releasing](./docs/releasing.md) — release checklist: tag → CI builds both executables
- [Agent connectivity (MCP)](./docs/mcp.md) — how harnesses (Claude Code, Codex, opencode) will use the memory
- [Sample dataset guide](./docs/sample-dataset.md) — what the demo corpus exercises

## Project layout

```
backend/    FastAPI service (connectors, ingestion, classification, RAG Q&A)
frontend/   React SPA (Vite) — Ask, Sources, Entities, Review, Settings, System
sample/     Mini-company demo corpus — connect it as a folder source
            (guide: docs/sample-dataset.md)
docs/       Product plan, architecture, packaging, releasing
```

## Run it

**Easiest (testers):** build once, ship one file — `.\build.ps1` (Windows)
produces `dist/AnchorCore.exe` (bundles the UI, auto-starts Ollama, data in
`~/.anchorcore`). Pushing a `v*` tag builds Windows + macOS executables
automatically (see `docs/releasing.md`).

**Developers:** `.\start.ps1` (or `./start.sh`) — creates the venv + `.env`
on first run, applies Alembic migrations, starts Ollama, boots the backend at
http://localhost:8000. Add `-Dev` for the Vite dev server at :5173.

**Prerequisites:** Python 3.12+, Node 20+, [Ollama](https://ollama.com):

```bash
ollama pull llama3.2:3b       # classifier
ollama pull nomic-embed-text  # embeddings
```

**Tests** (from `backend/`): `python -m pytest tests -q` — catalog:
[backend/tests/README.md](./backend/tests/README.md)

## Model configuration (Settings tab — no restart)

All model providers are configured in-app (runtime-mutable, DB-backed
`app_settings` overriding `.env` defaults; secrets in the OS keychain, never
in DB/logs):

| Section | Local | Cloud |
|---|---|---|
| Classification | Ollama (`llama3.2:3b`) | any OpenAI-compatible + key |
| Embeddings | Ollama (`nomic-embed-text`) | any OpenAI-compatible + key |
| Answer (Q&A) | Ollama | any OpenAI-compatible + key, reasoning effort (none/low/medium/high) |

Each section: provider dropdown, API key, base URL, model, **Test connection**.
`.env` remains the default layer — see `backend/.env.example` for all keys.

## How knowledge is built

```
Sources (folder, Jira) -> extract text -> classify -> entities + window context
                                          -> embed chunks (sqlite-vec)
                                          -> FTS5 keyword index (chunks_fts)
```

- **Document-aware classification**: one cheap call per document detects its
  type (standards/runbook/meeting/decision_log/prd/general); the extraction
  prompt adapts — e.g. regulatory standards produce only `note` entities
  (rules/definitions), never spurious "decision"/"action" labels.
- **Reviewable entities**: every entity stores the exact classifier input
  window (`window_text`) — the Review tab shows "what the classifier saw"
  behind each low-confidence item.
- **Cheap reclassification**: 16k-char windows + per-window content hashes —
  unchanged windows are skipped (a reclassify of unchanged content makes ~0
  classifier calls). Jobs are cancellable via **Stop** in the Sources tab.
- **Parallel throughput (B11)**: classifier windows run concurrently
  (`ANCHOR_CLASSIFIER_CONCURRENCY`, default 4); throughput stats in System tab.

**Retrieval (B12/B12.1):** chunks are cleaned (page numbers, repeated headers,
encoding artifacts) and split at section headings; Q&A fuses vector search
with SQLite FTS5 keyword scores via **reciprocal rank fusion** (RRF, k=60),
then applies age decay and a per-file diversity cap, expands winning chunks
with neighboring sections, and dedupes near-identical chunks. Config:
`ANCHOR_RETRIEVAL_KEYWORD_WEIGHT` (1.0), `ANCHOR_RETRIEVAL_MAX_PER_SOURCE` (3),
`ANCHOR_RETRIEVAL_AGE_HALFLIFE_DAYS` (365), `ANCHOR_RETRIEVAL_CONTEXT_WINDOW`
(1).

**Follow-up questions:** the Ask tab is a chat — follow-ups ("show the
movements for it") are rewritten into standalone queries using conversation
history, and the conversation is passed to generation. **Clear context**
resets the thread.

## Behavior without models

- No Ollama → classification falls back to rule-based; answers fall back to keyword context.
- No `ANCHOR_ANSWER_API_KEY` and cloud base URL → answers return matching context instead of LLM text.
- Everything degrades gracefully; add Ollama or a model key (Settings tab) to unlock the full experience.

## API surface (v1)

- `POST /sources` — connect a folder (path) or Jira (base_url, email, token, project)
- `POST /sources/{id}/sync`, `POST /sources/{id}/reclassify` — run ingestion now (returns 202 + job id)
- `GET /sources/jobs`, `GET /sources/jobs/{id}`, `GET /sources/jobs/running` — job progress + history (running/done/failed/cancelled)
- `POST /sources/jobs/{id}/cancel` — stop a running job
- `GET /entities`, `PATCH /entities/{id}` — browse and review (verify/dispute/reclassify)
- `GET /review/low-confidence`, `GET /review/duplicates`, `POST /review/merge` — review queue (entities carry `window_text` for review context)
- `POST /qa` — ask (optionally with `history` turns), get answer with section-level citations
- `GET /settings`, `PUT /settings` — runtime model config (secrets masked)
- `POST /settings/test-connection` — verify ollama/classifier/embedder/answer providers
- `GET /system/status`, `GET /system/errors`, `GET /system/logs[/{file}]` — health, errors, log download

## Data model (SQLite + Alembic migrations)

| Table | Notes |
|---|---|
| `sources` / `ingested_items` | connectors; items carry `doc_type` + `window_hashes` (cheap reclassify) |
| `entities` | kinds decision/document/action/note; `window_text`/`window_index` = classifier input; status verified/disputed/stale |
| `relationships` | typed links (supersedes/depends_on/owns/blocks) |
| `chunks` | full-doc + entity chunks, embeddings (float32 blobs) |
| `chunks_fts` | FTS5 keyword index, kept in sync by triggers |
| `merge_actions` | duplicate proposals + decisions |
| `jobs` | sync/reclassify progress + history |
| `system_events` | structured error/audit trail |
| `app_settings` | runtime overrides (secrets live in the OS keychain, not here) |
