# AnchorCore

Connect your knowledge to any AI model. An AI memory layer / knowledge operating system for teams.

## Docs

- [Product Plan](./docs/product-plan.md) — what we're building, for whom, and why
- [Architecture](./docs/architecture.md) — system view + architecture diagram
- [Packaging](./docs/packaging.md) — macOS distribution plan

## Project layout

```
backend/    FastAPI service (connectors, ingestion, classification, RAG Q&A)
frontend/   React SPA (Vite) — Ask, Sources, Entities, Review
sample/     Mini-company demo corpus — connect it as a folder source
            (guide: docs/sample-dataset.md)
docs/       Product plan and architecture
```

## Run locally

**Prerequisites:** Python 3.12+, Node 20+, and [Ollama](https://ollama.com) (optional but recommended):

```bash
ollama pull llama3.2:3b       # classifier
ollama pull nomic-embed-text  # embeddings
```

**Large-document throughput (B11):** classification windows run in parallel
(4 by default, configurable via `ANCHOR_CLASSIFIER_CONCURRENCY`). For the
parallelism to actually speed things up, the Ollama server must agree:

```bash
# Windows: set as user env vars, then restart Ollama
setx OLLAMA_NUM_PARALLEL 4
setx OLLAMA_CONTEXT_LENGTH 8192   # windows need ~2.5k tokens; default wastes KV cache
setx OLLAMA_KEEP_ALIVE 30m        # avoid model unload churn between long jobs
# macOS/Linux: export the same vars before running `ollama serve`
```

Throughput (windows processed, avg latency) is visible in the System tab.

**Retrieval (B12):** chunks are cleaned (page numbers, repeated headers,
encoding artifacts) and split at section headings; Q&A fuses vector search
with SQLite FTS5 keyword scores via **reciprocal rank fusion** (RRF, k=60),
then applies age decay and a per-source diversity cap, and expands winning
chunks with neighboring sections. Config: `ANCHOR_RETRIEVAL_KEYWORD_WEIGHT`
(list weight, default 1.0), `ANCHOR_RETRIEVAL_MAX_PER_SOURCE` (3),
`ANCHOR_RETRIEVAL_AGE_HALFLIFE_DAYS` (365), `ANCHOR_RETRIEVAL_CONTEXT_WINDOW`
(1). After upgrading, run **Reclassify** on existing sources to rebuild chunks
with cleaning + heading boundaries (`chunks_fts` is created by the Alembic
migration automatically).

**One command** (Windows `.\start.ps1` / macOS+Linux `./start.sh`):

```bash
./start.ps1       # Windows — or ./start.sh on mac/linux
```

This is the single supported entry point: it kills orphaned processes on the
port, starts the local Ollama server if it isn't running (skip with
`-NoOllama` / `--no-ollama`), creates the venv and `backend/.env` on first
run, applies Alembic migrations, and boots the backend (serving the built UI
at http://localhost:8000). The app prints its effective config at boot (DB
path, models, Ollama reachability). Add `-Dev` / `--dev` to also start the
Vite dev server at http://localhost:5173.

Manual steps (equivalent, for reference):

**Backend** (from `backend/`):

```bash
python -m venv .venv
.venv/Scripts/activate        # Windows (.venv/bin/activate on mac/linux)
pip install -r requirements.txt
cp .env.example .env          # set ANCHOR_ANSWER_API_KEY for cloud answers,
                              # or point ANCHOR_ANSWER_BASE_URL at Ollama for local answers
alembic upgrade head          # schema migrations (SQLite history in alembic/versions)
uvicorn app.main:app --port 8000
```

**Frontend** (from `frontend/`):

```bash
npm install
npm run dev                   # dev server on :5173, proxies API to :8000
```

Or `npm run build` — the built `dist/` is served automatically by the backend at http://localhost:8000.

**Tests** (from `backend/`): `python -m pytest tests -q` — catalog: [backend/tests/README.md](./backend/tests/README.md)

**CI:** GitHub Actions runs backend tests + frontend build on every push
(see `.github/workflows/ci.yml`).

## Sync & jobs

`POST /sources/{id}/sync` and `/reclassify` are asynchronous: they return
`202` with a job immediately, run in the background, and report progress via
`GET /sources/jobs/{id}` (status, processed/total, result, error) and history
via `GET /sources/jobs?source_id=N`. Long operations never block or look
frozen.

## Behavior without models

- No Ollama → classification falls back to rule-based; answers fall back to keyword context.
- No `ANCHOR_ANSWER_API_KEY` and cloud base URL → answers return matching context instead of LLM text.
- Everything degrades gracefully; add Ollama + a model key (or point the answer engine at Ollama) to unlock the full experience.

## API surface (v1)

- `POST /sources` — connect a folder (path) or Jira (base_url, email, token, project)
- `POST /sources/{id}/sync`, `POST /sources/{id}/reclassify` — run ingestion now (returns 202 + job id; polls/syncs run in background)
- `GET /sources/jobs`, `GET /sources/jobs/{id}` — job progress + history (running/done/failed, processed/total, result)
- `GET /entities`, `PATCH /entities/{id}` — browse and review (verify/dispute/reclassify)
- `GET /review/low-confidence`, `GET /review/duplicates`, `POST /review/merge` — review queue
- `POST /qa` — ask, get answer with section-level citations
