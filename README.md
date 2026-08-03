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
sample/     Example docs for local testing
docs/       Product plan and architecture
```

## Run locally

**Prerequisites:** Python 3.12+, Node 20+, and [Ollama](https://ollama.com) (optional but recommended):

```bash
ollama pull llama3.2:3b       # classifier
ollama pull nomic-embed-text  # embeddings
```

**Backend** (from `backend/`):

```bash
python -m venv .venv
.venv/Scripts/activate        # Windows (.venv/bin/activate on mac/linux)
pip install -r requirements.txt
cp .env.example .env          # set ANCHOR_ANSWER_API_KEY for cloud answers,
                              # or point ANCHOR_ANSWER_BASE_URL at Ollama for local answers
uvicorn app.main:app --port 8000
```

**Frontend** (from `frontend/`):

```bash
npm install
npm run dev                   # dev server on :5173, proxies API to :8000
```

Or `npm run build` — the built `dist/` is served automatically by the backend at http://localhost:8000.

**Tests** (from `backend/`): `python -m pytest tests -q`

## Behavior without models

- No Ollama → classification falls back to rule-based; answers fall back to keyword context.
- No `ANCHOR_ANSWER_API_KEY` and cloud base URL → answers return matching context instead of LLM text.
- Everything degrades gracefully; add Ollama + a model key (or point the answer engine at Ollama) to unlock the full experience.

## API surface (v1)

- `POST /sources` — connect a folder (path) or Jira (base_url, email, token, project)
- `POST /sources/{id}/sync` — run ingestion now (polls run on a schedule in background)
- `GET /entities`, `PATCH /entities/{id}` — browse and review (verify/dispute/reclassify)
- `GET /review/low-confidence`, `GET /review/duplicates`, `POST /review/merge` — review queue
- `POST /qa` — ask, get answer with section-level citations
