# AnchorCore

Connect your knowledge to any AI model. An AI memory layer / knowledge operating system for teams.

## Docs

- [Product Plan](./docs/product-plan.md)
- [Architecture Overview](./docs/architecture.md)
- [macOS Packaging Plan](./docs/packaging.md)

## Project layout

```
backend/    FastAPI service (upload, extraction, classification, storage)
frontend/   Next.js UI (upload + knowledge object explorer)
docs/       Product plan and architecture
```

## Run locally

**Backend** (from `backend/`):

```bash
python -m venv .venv
.venv/Scripts/activate        # Windows
pip install -r requirements.txt
cp .env.example .env          # then edit if needed
uvicorn app.main:app --reload --port 8000
```

**Frontend** (from `frontend/`):

```bash
npm install
npm run dev
```

Open http://localhost:3000.

### Classifier

- Defaults to an OpenAI-compatible endpoint at `http://localhost:11434/v1` (Ollama).
- Run `ollama pull llama3.2` (or set `ANCHOR_CLASSIFIER_MODEL`) for LLM classification.
- Without an LLM reachable, the service falls back to a rule-based classifier so the slice still works.

### Database

- Defaults to SQLite (`backend/anchorcore.db`), no setup needed.
- For PostgreSQL, run `docker compose up -d db` and set `ANCHOR_DATABASE_URL=postgresql+psycopg://anchor:anchor@localhost:5432/anchorcore` (requires `psycopg2`/`psycopg`).
