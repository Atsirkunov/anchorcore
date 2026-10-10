# AnchorCore

Your memory — finally searchable.

AnchorCore turns your folders, Drive, Jira, Linear and JSON APIs into a private AI knowledge base
on your own machine. Ask "what was decided about X, and why?" — get a cited answer
that shows exactly where it came from.

## Download

Free for personal use. No account, no Docker, no setup:

- [Download for Windows](https://github.com/Atsirkunov/anchorcore/releases/latest/download/AnchorCore-windows.zip)
- [Download for macOS](https://github.com/Atsirkunov/anchorcore/releases/latest/download/AnchorCore-macos.zip)

Unzip, run, and the app opens in your browser. Your data stays in `~/.anchorcore`
on your machine — nothing leaves it unless you configure a cloud model.

> First launch: on macOS right-click → Open (the app isn't Apple-notarized yet);
> on Windows click "More info → Run anyway" on the SmartScreen prompt.

## 60-second start

1. Download and run the app (above).
2. Sources → Local folder → point at the repo's `sample/` demo corpus (grab the
   folder from GitHub; release zips ship the app only — or use your own) → Sync.
3. Ask: "What was decided about security transfers, and why?" → cited answer.

Tip: install [Ollama](https://ollama.com) and pull `llama3.2:3b` +
`nomic-embed-text` for full hybrid search — without it, keyword search still works.

## Editions

- **Personal** — this app. Free forever for personal use (see [License](#license)).
- **Team (self-hosted)** — one Docker container in your VPC, one-time platform
  license (perpetual, incl. 1 year of support/updates). See
  [hosting/README.md](./hosting/README.md).
- **Hosted** — coming soon.

## License

Free for personal use under [LICENSE.md](./LICENSE.md) (source-available).
Team and commercial use requires a paid license.

## Docs

- [Product Plan](./docs/product-plan.md) — what we're building, for whom, and why (+ full backlog)
- [Architecture](./docs/architecture.md) — system view + architecture diagram
- [Packaging](./docs/packaging.md) — Rust binary + MCP sidecar zips for Windows, macOS and Linux
- [Releasing](./docs/releasing.md) — release checklist: tag → CI builds all platform zips
- [Agent connectivity (MCP)](./docs/mcp.md) — how harnesses (Claude Code, Codex, opencode, Cursor) use the memory
- [v2/v3 business scoping](./docs/v2v3-scope.md) — website, hosting, sharing, pricing, free tier
- [Design system](./docs/design-system.md) — color tokens, typography, messaging, website approach (single source for app + site)
- [Rust port evaluation](./docs/rust-port.md) — is a Rust backend worth it? (distribution vs LLM latency)
- [Sample dataset guide](./docs/sample-dataset.md) — what the demo corpus exercises

## Project layout

```
rust/       Rust service (Axum) — connectors, ingestion, classification, RAG Q&A — shipped artifact (1.0.13, single source rust/Cargo.toml)
frontend/   React SPA (Vite) — Ask, Sources, Entities, Review, Settings, System
sample/     Mini-company demo corpus — connect it as a folder source
            (guide: docs/sample-dataset.md)
hosting/    Hosted skeleton — Python FastAPI image, Postgres (docker-compose), env-driven
            (guide: hosting/README.md) — stays Python per R10.7 until hosted is ported
backend/    Archived on tag archive/python-final — frozen Python backend; hosting image + conformance fetch it. Do not use for new dev.
docs/       Product plan, architecture, packaging, releasing
```

## Run it

**Easiest:** download the app ([Windows / macOS links above](#download), or the
[latest Release](https://github.com/Atsirkunov/anchorcore/releases/latest)) — unzip,
run, done. Each build bundles the UI, auto-starts Ollama, data in `~/.anchorcore`.
To build it yourself: `.\build.ps1` (Windows) or `./build.sh` (macOS) from the repo
root; pushing a `v*` tag builds all platform zips automatically (see `docs/releasing.md`).

**Developers (Rust — shipped):** `cargo run -p anchorcore -- --port 8000 --data-dir ~/.anchorcore` — applies migrations, starts Ollama, boots at http://localhost:8000. Add `cargo run -p anchorcore -- --port 8123` for conformance vs Python. Frontend dev: `cd frontend && npm run dev` at :5173 (proxies to :8000).

**Legacy Python (archived):** the Python backend left the tree — it's frozen on tag `archive/python-final` (hosting image + conformance suite fetch it from there). To run it: `git checkout archive/python-final -- backend`, then `.\start.ps1` (or `./start.sh`).

**Hosted (skeleton):** `docker compose -f hosting/docker-compose.yml up --build` — Python FastAPI image against Postgres (`pgvector/pg16`, see `hosting/README.md`). **R10.7 decision:** `hosting/` stays Python (Postgres/pgvector) — Rust is the local/packaged artifact (SQLite + `frontend/dist` embedded, `9.8M`, `codesign`); Rust Postgres is deferred (would need `deadpool` + `pgvector` migration, or SQLite-file volume on hosted).

**Prerequisites:** Rust (stable) + Node 20+ for local dev, Python 3.12 only for `hosting/`/conformance, [Ollama](https://ollama.com):

```bash
ollama pull llama3.2:3b       # classifier
ollama pull nomic-embed-text  # embeddings
```

**Tests — Rust is source:** `cargo test -p anchorcore` (all green, `cargo check` clean) + `cargo build --release` embeds `frontend/dist` (`9.8M`). Conformance vs Python: `PYTHONPATH=backend ANCHOR_TEST_RUST_URL=http://127.0.0.1:8123 pytest -q --ignore=backend/tests/test_mcp.py` (honest accounting). Legacy Python unit: `python -m pytest backend/tests -q` (`backend/tests/README.md`).

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
`.env` remains the default layer — the Settings tab is the normal way to configure it; env keys are listed in `hosting/.env.example` (full legacy list on tag `archive/python-final`).

## How knowledge is built

```
Sources (folder, Jira) -> extract text -> clean -> chunk -> classify -> entities + window context
                                          -> embed chunks (sqlite-vec)
                                          -> FTS5 keyword index (chunks_fts)
                                          -> (Phase 14) section tree + summary nodes + tags
```

- **Cleaning → chunking (B5/B12/B35):** text is cleaned (`clean_text` strips control chars, page numbers, repeated headers) then section-aware chunked: heading detection (`§434`, `4.2.1`, `Article 12`, ALL-CAPS) creates hard boundaries at `backend/app/chunking.py:49` / `rust/crates/anchorcore/src/chunking.rs:68`; paragraphs pack up to `ANCHOR_CHUNK_MAX_CHARS=1600`, oversized paras fall back to fixed `800/100` slices `chunk_text`. Entity summaries chunk separately at `800/100`. Classifier windows are `16000` chars with 50% overlap (`classify_windows`), hash-deduped for cheap reclassify.
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
  thread phrased it as "what's the timeout?". An IDF gate (`0.15` signal) skips low-signal
  filler from vector search (it stays keyword-findable in FTS5).

**Retrieval (B12/B12.1):** chunks are cleaned then split at section headings; Q&A fuses vector search (sqlite-vec `vec_chunks` vec0, `k=top_k*4`) with FTS5 bm25 via **RRF** (`score=Σ w/(60+rank)`, `k=60`), then age decay `0.5^(age/halflife)`, per-item diversity cap (3, relaxed for single-file corpora), `±1` neighbor `expand_context`, and near-duplicate dedupe. Config:
`ANCHOR_RETRIEVAL_KEYWORD_WEIGHT` (1.0), `ANCHOR_RETRIEVAL_MAX_PER_SOURCE` (3),
`ANCHOR_RETRIEVAL_AGE_HALFLIFE_DAYS` (365), `ANCHOR_RETRIEVAL_CONTEXT_WINDOW`
(1). Planner picks `hybrid` + `who_knows`; graph walk (B32) adds 1–2 hop related entities.

**Scale — hierarchical TOC (Phase 14, shipped in 1.0.12):** flat chunks don't scale to 150k+ chunks (5k docs, `scripts/build_business_corpus.py`). Before, `chunk_document` derived `sections` then discarded the tree — no `section_id`/`path`/`level`. Now `sections` (parent/level/path/summary + `summary_embedding`) + dynamic `tags`/`chunk_tags` (reuse if `cosine>0.82` else create) are persisted (`migrations/11_sections.sql`, `12_tags.sql`). Retrieval is coarse-to-fine: TOC/tag SQL prune → summary-node vector search (~5k sections) → leaf vec0 `WHERE c.section_id IN (:top5)` within winning sections (`retrieval.rs:836` `toc_search`), avoiding flat `vector_search_scan` fallback. Endpoints `GET /sections?item_id=&source_id=`, `GET /sections/:id/chunks`, `GET /tags`, `GET /tags/:id/chunks`; UI shows `path` breadcrumb `Art 12 › 12.3` + tag chips + `tag_reuse_threshold` in Settings. See `docs/architecture.md:3` + `docs/guides/hierarchical-toc.md` + `rust/BACKLOG.md:Phase 14`.

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
- `GET /pii/config`, `PUT /pii/config`, `GET /pii/review`, `POST /pii/review/{id}`, `POST /pii/scan/{source}`, `GET /pii/item/{id}` — PII config + review (B30, Direct= pii / Indirect= sensitive, source-level for v1)
 - `GET /auth/status`, `POST /auth/signup`, `POST /auth/login`, `GET /auth/me` — hosted auth (B40, `ANCHOR_AUTH_SECRET` enables JWT; local stays no-auth)
 - `GET /settings`, `PUT /settings` — runtime model config (`tag_reuse_threshold` 0.82, `PUT` persists to `app_settings`, `tags.rs:50` reads env `ANCHOR_TAG_REUSE_THRESHOLD` → DB fallback)
 - `POST /settings/test-connection` — verify ollama/classifier/embedder/answer providers
 - `GET /system/status`, `GET /system/errors`, `GET /system/logs[/{file}]`, `GET /system/onboarding` — health, errors, log download, wizard trigger (B8, skippable)
 - `GET /sections?item_id=&source_id=`, `GET /sections/:id/chunks`, `GET /tags`, `GET /tags/:id/chunks` — hierarchical TOC & dynamic taxonomy (Phase 14, `sections.rs:1`, `tags.rs:1`)

## Data model (SQLite local / Postgres hosted + Alembic migrations)

| Table | Notes |
|---|---|
| `sources` / `ingested_items` | connectors; items carry `doc_type` + `window_hashes` (cheap reclassify) + `distill_hashes` (B18); `sources.label` B39 (internal|public|sensitive|pii) |
| `projects` / `project_sources` | source bundles for scoped search (B15) |
| `entities` | kinds decision/document/action/note; `window_text`/`window_index` = classifier input; status verified/disputed/stale |
| `relationships` | typed links (supersedes/depends_on/owns/blocks) |
| `chunks` | `kind` = document/entity/distilled (+ section_summary/doc_summary Phase 14); `section_id`/`level`/`path` (Phase 14); `is_pii` + `pii_categories` (B30); embeddings (float32 blobs); FTS5 `chunks_fts` + vec0 `vec_chunks` (B33) kept in sync by triggers (SQLite-only, skip on Postgres) |
| `sections` / `tags` / `chunk_tags` | Phase 14: hierarchical TOC (`sections` parent/level/path/summary + `summary_embedding`) + dynamic taxonomy (`tags` embedding + count, many:many via `chunk_tags`; reuse if `cosine>0.82` else create) |
| `chunks_fts` / `vec_chunks` | FTS5 keyword index / vec0 cosine index (SQLite), triggers sync; Postgres skips (fallback to Python scan) |
| `merge_actions` | duplicate proposals + decisions |
| `jobs` | sync/reclassify progress + history |
| `system_events` | structured error/audit trail |
| `app_settings` | runtime overrides (secrets live in the OS keychain, not here; PII custom words/disabled categories B30) |
| `users` | hosted auth (B40, email unique, pbkdf2 hash) — local stays single-user no-auth |
