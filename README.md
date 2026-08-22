# AnchorCore

Connect your knowledge to any AI model. Your memory — finally searchable. — personal or team, same local app (see [Design system](./docs/design-system.md#51-website-hero--a-chosen--personal--company-split) for the two-track language).

## Docs

- [Product Plan](./docs/product-plan.md) — what we're building, for whom, and why (+ full backlog)
- [Architecture](./docs/architecture.md) — system view + architecture diagram
- [Packaging](./docs/packaging.md) — Windows exe + macOS app (both built from one spec)
- [Releasing](./docs/releasing.md) — release checklist: tag → CI builds both executables
- [Agent connectivity (MCP)](./docs/mcp.md) — how harnesses (Claude Code, Codex, opencode) will use the memory
- [v2/v3 business scoping](./docs/v2v3-scope.md) — website, hosting, sharing, pricing, free tier
- [Design system](./docs/design-system.md) — color tokens, typography, messaging, website approach (single source for app + site)
- [Rust port evaluation](./docs/rust-port.md) — is a Rust backend worth it? (distribution vs LLM latency)
- [Sample dataset guide](./docs/sample-dataset.md) — what the demo corpus exercises

## Project layout

```
backend/    FastAPI service (connectors, ingestion, classification, RAG Q&A)
frontend/   React SPA (Vite) — Ask, Sources, Entities, Review, Settings, System
sample/     Mini-company demo corpus — connect it as a folder source
            (guide: docs/sample-dataset.md)
hosting/    Hosted skeleton — same image, Postgres (docker-compose), env-driven
            (guide: hosting/README.md)
docs/       Product plan, architecture, packaging, releasing
```

## Run it

**Easiest (testers):** build once, ship one file — `.\build.ps1` (Windows)
produces `dist/AnchorCore.exe`, or `./build.sh` (macOS) produces
`dist/AnchorCore` (ad-hoc signed). Each bundles the UI, auto-starts Ollama,
data in `~/.anchorcore`. Pushing a `v*` tag builds Windows + macOS
executables automatically (see `docs/releasing.md`).

**Developers:** `.\start.ps1` (or `./start.sh`) — creates the venv + `.env`
on first run, applies Alembic migrations, starts Ollama, boots the backend at
http://localhost:8000. Add `-Dev` for the Vite dev server at :5173.

**Hosted (skeleton):** `docker compose -f hosting/docker-compose.yml up --build` — Python FastAPI image against Postgres (`pgvector/pg16`, see `hosting/README.md`). **R10.7 decision:** `hosting/` stays Python (Postgres/pgvector) — Rust is the local/packaged artifact (SQLite + `frontend/dist` embedded, `9.8M`, `codesign`); Rust Postgres is deferred (would need `deadpool` + `pgvector` migration, or SQLite-file volume on hosted).

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
- **Distillation (B18)**: chat-like sources (meetings, Slack exports) are
  normalized into searchable Q&A units (`Q: … A: …`) that embed well — so
  "how long does the idempotency key last?" is answered even when the raw
  thread phrased it as "what's the timeout?". An IDF gate skips low-signal
  filler from vector search (it stays keyword-findable in FTS5).

**Retrieval (B12/B12.1):** chunks are cleaned (page numbers, repeated headers,
encoding artifacts) and split at section headings; Q&A fuses vector search
with SQLite FTS5 keyword scores via **reciprocal rank fusion** (RRF, k=60),
then applies age decay and a per-file diversity cap, expands winning chunks
with neighboring sections, and dedupes near-identical chunks. Config:
`ANCHOR_RETRIEVAL_KEYWORD_WEIGHT` (1.0), `ANCHOR_RETRIEVAL_MAX_PER_SOURCE` (3),
`ANCHOR_RETRIEVAL_AGE_HALFLIFE_DAYS` (365), `ANCHOR_RETRIEVAL_CONTEXT_WINDOW`
(1).

**Planner → Executor → Synthesis (B17):** a lightweight planner picks the
retrieval tools per query — `hybrid` (vector + keyword) always, plus a
`who_knows` tool for ownership/expertise questions ("who owns X?"). The
executor runs them (one shared embedding call), normalizes each tool's hits
into one evidence bundle, and synthesis RRF-fuses them into the final context.
An LLM planner can replace the heuristic rules later without changing the
contract.

**Graph-based retrieval (B32):** after RRF picks the winning entities, the
pipeline walks the entity graph (`relationships`: supersedes/depends_on/owns/
blocks/related) 1-2 hops and pulls connected entities into the answer context
and citations — so "what supersedes this?" answers even when the words don't
co-occur in both documents.

**Scoped search via projects (B15):** group sources into named projects (a
source can be in several); pick one in the header to scope every question to
that project's sources, or "All sources". The default project is marked ★.

**Follow-up questions:** the Ask tab is a chat — follow-ups ("show the
movements for it") are rewritten into standalone queries using conversation
history, and the conversation is passed to generation. **Clear context**
resets the thread.

## Behavior without models

- No Ollama → classification falls back to rule-based; answers fall back to keyword context.
- No `ANCHOR_ANSWER_API_KEY` and cloud base URL → answers return matching context instead of LLM text.
- Everything degrades gracefully; add Ollama or a model key (Settings tab) to unlock the full experience.

## API surface (v1 — env-driven: SQLite local, Postgres hosted via `hosting/`)

- `POST /sources` — connect a folder (path, `label` internal|public|sensitive|pii B39) or Jira (base_url, email, token, project)
- `GET /sources/{id}/config` — source config (secrets masked) — shown as folder path / Jira details in the Sources tab
- `POST /sources/{id}/sync`, `POST /sources/{id}/reclassify` — run ingestion now (returns 202 + job id)
- `GET /sources/jobs`, `GET /sources/jobs/{id}`, `GET /sources/jobs/running` — job progress + history (running/done/failed/cancelled)
- `POST /sources/jobs/{id}/cancel` — stop a running job
- `DELETE /sources/{id}` — remove a source and cascade-delete its items/entities/chunks
- `GET /entities`, `GET /entities/{id}`, `PATCH /entities/{id}` — browse and review (verify/dispute/reclassify)
- `GET /entities/{id}/related`, `GET /entities/{id}/disputes`, `GET /entities/{id}/context` — related graph, dispute audit (B3), expanded review context with neighbours + highlight (B27)
- `GET /review/low-confidence`, `GET /review/duplicates` (paginated `limit`/`offset`/`kind` B38), `POST /review/merge` — review queue (entities carry `window_text` for review context)
- `POST /qa` — ask (optionally with `history` turns, `project_id` scope, `public_only` B30), get answer with section-level citations; `POST /qa/public` — share-safe, public sources only (B30)
- `GET/POST /projects`, `GET/PATCH/DELETE /projects/{id}`, `GET /projects/default` — project bundles for scoped search
- `GET /pii/config`, `PUT /pii/config`, `GET /pii/review`, `POST /pii/review/{id}`, `POST /pii/scan/{source}` — PII config + review (B30, Direct= pii / Indirect= sensitive, source-level for v1)
- `GET /auth/status`, `POST /auth/signup`, `POST /auth/login`, `GET /auth/me` — hosted auth (B40, `ANCHOR_AUTH_SECRET` enables JWT; local stays no-auth)
- `GET /settings`, `PUT /settings` — runtime model config (secrets masked)
- `POST /settings/test-connection` — verify ollama/classifier/embedder/answer providers
- `GET /system/status`, `GET /system/errors`, `GET /system/logs[/{file}]`, `GET /system/onboarding` — health, errors, log download, wizard trigger (B8, skippable)

## Data model (SQLite local / Postgres hosted + Alembic migrations)

| Table | Notes |
|---|---|
| `sources` / `ingested_items` | connectors; items carry `doc_type` + `window_hashes` (cheap reclassify) + `distill_hashes` (B18); `sources.label` B39 (internal|public|sensitive|pii) |
| `projects` / `project_sources` | source bundles for scoped search (B15) |
| `entities` | kinds decision/document/action/note; `window_text`/`window_index` = classifier input; status verified/disputed/stale |
| `relationships` | typed links (supersedes/depends_on/owns/blocks) |
| `chunks` | `kind` = document/entity/distilled; `is_pii` + `pii_categories` auto-flagged (B30); embeddings (float32 blobs); FTS5 `chunks_fts` + vec0 `vec_chunks` (B33) kept in sync by triggers (SQLite-only, skip on Postgres) |
| `chunks_fts` / `vec_chunks` | FTS5 keyword index / vec0 cosine index (SQLite), triggers sync; Postgres skips (fallback to Python scan) |
| `merge_actions` | duplicate proposals + decisions |
| `jobs` | sync/reclassify progress + history |
| `system_events` | structured error/audit trail |
| `app_settings` | runtime overrides (secrets live in the OS keychain, not here; PII custom words/disabled categories B30) |
| `users` | hosted auth (B40, email unique, pbkdf2 hash) — local stays single-user no-auth |
