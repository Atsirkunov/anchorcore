# AnchorCore — Architecture Overview

> Stack: **Rust (Axum) · React (Vite) · SQLite locally (sqlite-vec vec0 +
> FTS5) / Postgres for hosted (`hosting/`, Python, `pgvector/pg16`) · Ollama
> or BYO cloud LLM APIs** — env-driven. **Rust is the shipped backend;
> Python `backend/` is deprecated except `hosting/`** (decision R10.7).

---

## 1. Runtime Model: Local-First, Hosted Env-Driven

One local process = the entire product. No Docker, no sidecar services, no
setup. **Local:** Rust binary `anchorcore` (`frontend/dist` embedded,
SQLite `vec0` + FTS5). **Hosted:** `hosting/` Docker + Postgres stays a
Python FastAPI image, env-driven via `ANCHOR_DATABASE_URL` — the SQLite
FTS5/vec0 triggers skip on Postgres (falls back to scan). See
`hosting/README.md` + `docs/v2v3-scope.md`.

## 2. System Diagram

```mermaid
flowchart TB
    subgraph laptop["User's Laptop (Windows/macOS)"]
        direction TB
        browser["Browser UI<br/>(React SPA, localhost)"]
        api["AnchorCore App<br/>Rust Axum (single process)"]
        api --- connectors["Connectors<br/>Folder · Drive · Jira · Linear · REST"]
        api --- pipeline["Ingestion Pipeline<br/>type detect → classify → distill → entities"]
        api --- rag["Answer Engine<br/>planner → executor → RRF → graph → cited answer"]
        api --- review["Review API<br/>low-confidence · duplicates"]
        api --- projects["Projects API<br/>scoped search"]
        api --- jobs["Job Manager<br/>background jobs · cancel"]
        db[("Entity Graph<br/>SQLite + sqlite-vec + FTS5")]
        settings[("app_settings<br/>runtime overrides")]
        secrets["Secrets<br/>OS Keychain"]
        api --- db
        api --- settings
        api --- secrets
        browser --- api
        ollama["Ollama<br/>classifier + embed models"]
        api --- ollama
    end

    tickets["Jira · Linear · REST APIs"]
    drive["Google Drive"]
    folder["Local folder<br/>(watched)"]
    cloud["BYO LLM API<br/>(OpenAI-compatible)"]

    connectors --- folder
    connectors --- drive
    connectors --- tickets
    pipeline --- cloud
    rag --- cloud
```

## 3. Data Flow — Ingestion

```mermaid
flowchart LR
    watcher["Folder watcher<br/>fs events + hourly scan"] --> extract["Extract text"]
    apis["Drive · Jira · Linear · REST<br/>poll, incremental cursors"] --> extract
    extract --> hash["Hash dedup"]
    hash --> type["Document-type pre-pass<br/>(1 cheap call, cached)"]
    type --> classify["Classify windows<br/>(local Ollama or cloud, parallel)"]
    classify --> entities["Entities + window context<br/>(SQLite)"]
    classify --> embed["Embed chunks<br/>(local or cloud)"]
    embed --> vec["sqlite-vec index"]
    classify --> fts["FTS5 keyword index"]
    entities --> review["Review UI<br/>context panel · confirm / merge / reclassify"]
```

**Deletion policy:** deleting a source cascade-deletes everything under it
(items, entities, chunks, jobs) via FK cascade. Re-syncing a changed document
replaces its entities/chunks. Retrieval always excludes `stale` entities —
and `disputed` ones unless opted back in.

**Chunking & cleaning:**

| Layer | Function | Config | Purpose |
|---|---|---|---|
| Cleaning | `clean_text` + repeated-line stripping | — | Strip control chars, page numbers, repeated headers/footers and normalize whitespace *before* chunking, so boundaries, embeddings and FTS tokens are clean |
| Entity summary | `chunk_text` (`chunking.py` / `chunking.rs`) | `ANCHOR_CHUNK_SIZE=800`, `ANCHOR_CHUNK_OVERLAP=100` (chars, UTF-8 safe) | Fixed 800/100 slices of each entity summary → `chunks kind=entity` |
| Full-document | `chunk_document` | `ANCHOR_CHUNK_MAX_CHARS=1600` | Section-aware: hard split at headings (`§434`, `4.2.1`, `Article 12`, ALL-CAPS), paragraphs packed per section up to 1600 chars, fixed-slice fallback only for oversized paragraphs |
| Classifier windows | `classify_windows` | `ANCHOR_CLASSIFY_WINDOW_CHARS=16000`, 50% overlap | Overlapping windows for the classifier — not stored as chunks, but governs hash dedup and cost |

The pipeline runs clean → strip → chunk → store (`Chunk(kind=document)`,
old doc chunks deleted on re-sync). Known gap: Rust still only trims instead
of full `clean_text` (parity follow-up).

**Distillation:** chat-like windows (meetings, decision logs, general notes)
are normalized into searchable Q&A units stored as `kind='distilled'` chunks.
An IDF gate (`embed_min_signal=0.15`) skips low-signal filler from vector
embedding — it stays keyword-findable in FTS5. Distillation is hash-skipped
per window.

**Scale — hierarchical TOC (shipped 1.0.12):** documents keep their structure
now. Headings become a saved `sections` tree (parent/level/path/summary),
passages point at their section, and a `tags` taxonomy is built as documents
arrive (similar topics merged at 0.82 similarity). Retrieval goes
coarse-to-fine — tag prune, summary search over ~5k section summaries, then
leaf search inside the winning sections — instead of ranking all ~150k
chunks flat. Full detail: `docs/guides/hierarchical-toc.md`.

## 4. Data Flow — Q&A

```mermaid
sequenceDiagram
    participant U as User (browser)
    participant A as AnchorCore App
    participant V as sqlite-vec
    participant F as FTS5 (chunks_fts)
    participant O as Ollama
    participant M as BYO LLM API

    U->>A: "What was decided about X, and why?" (+ chat history, project scope)
    A->>A: rewrite follow-up into standalone query (if history)
    A->>A: resolve project scope → source ids
    A->>A: planner — pick retrieval tools (hybrid, who_knows)
    A->>O: embed query
    O-->>A: query vector
    A->>V: top-k similarity (scoped to project sources)
    A->>F: FTS5 keyword (bm25, scoped)
    A->>A: who_knows tool — owner/expertise entities
    A->>A: RRF fusion + age decay + diversity cap + context expansion
    A->>A: graph walk — connected entities join context + citations
    A->>M: answer prompt w/ sections (+ conversation)
    M-->>A: answer + citations
    A-->>U: answer + clickable source chips
```

### 4a. Access pattern — flat vs hierarchical

*Flat:* `ask` resolves project source ids → the planner picks tools (`hybrid`
always, `who_knows` on ownership cues) → the executor runs them with one
shared embedding call (`hybrid` = vec0 vector + FTS5 keyword) → fusion (RRF
`score=Σ w/(60+rank)`, age decay, per-item diversity cap, context expansion,
near-duplicate dedupe) → 1–2 hop graph walk → PII/sensitive gate for
unapproved providers → cited generation. Every retrieval path takes an
optional source-id scope; every hit carries chunk, entity, item, source and
score.

*Hierarchical (shipped 1.0.12):* the same pipeline with a TOC pre-stage
(`toc_search` before flat hybrid). Persisted sections + tags let retrieval
prune *before* vector search: tag/SQL filter, summary-node vector search over
~5k summaries, then leaf vec0 restricted to the top sections. Falls back to
flat hybrid when the TOC pass is empty. Context expansion reads same-section
siblings. Endpoints: `GET /sections`, `GET /tags`.

## 4b. Retrieval Design (informed by Cerebras' knowledge base)

Cerebras published how their internal KB serves 15k queries/day (2026-07).
Their design validated our hybrid approach and added concrete techniques we
adopted:

1. **Multi-scorer fusion, no single trusted scorer** — they run full-text,
   embeddings, and IDF-scored retrievers in parallel and fuse the ranked
   lists. We fuse FTS5 (bm25) + cosine via **reciprocal rank fusion (RRF)**:
   `score = Σ weight / (60 + rank)`. Consensus across scorers beats a single
   strong vote; no score normalization needed.
2. **Per-file diversity cap** — a document that matches broadly must not
   monopolize the top-k. After fusion, cap results per file (default 3);
   relaxed for single-file corpora so a big document's sections can surface.
3. **Age decay** — when relevance is otherwise equal, the newer hit wins. A
   recency multiplier is applied in fusion.
4. **Context expansion** — once winners are picked, pull the neighboring
   sections (heading, preconditions, caveats) that chunking split apart, so
   the LLM sees a complete section instead of a lonely paragraph.
5. **Near-duplicate chunk dedupe** — TOC entries duplicating body sections
   keep only the best-scoring copy.

Follow-up questions reuse retrieval with a **query-rewrite pass**: history is
sent with the question, a cheap LLM call rewrites it standalone, and the
conversation is included in generation.

**Planner → Executor → Synthesis:** a lightweight planner picks the
retrieval tools for each query (`hybrid` vector+FTS always; `who_knows` for
ownership/expertise questions). The executor runs them (one shared embedding
call), each tool returns a ranked hit list in a normalized evidence shape, and
synthesis RRF-fuses them into the final context. An LLM planner can replace the
heuristic rules later without changing the executor contract.

**Graph-based retrieval:** after RRF picks the winning entities, the
pipeline walks `relationships` 1–2 hops (`supersedes` > `depends_on` > `owns` >
`blocks` > `related`, hop decay, fan-out cap) and pulls connected entities'
summaries/chunks into the answer context as `[related]` sections + citations.
This answers "what supersedes this?" / "what depends on this decision?" that
lexical+vector fusion alone misses, because the connected knowledge doesn't
need to co-occur in the question's words. Pure SQL — no model calls.

Remaining backlog: a fuller *who_knows* ranking and per-user ACLs for hosted.
Scoped search via *projects* and PII gating both shipped.

## 5. Data Model — Uniform Entity Graph

Everything is an entity. Contradictions stay external to the baseline object.

```mermaid
erDiagram
    INGESTED_ITEMS {
        int id PK
        int source_id FK
        string external_id
        string title
        text text "raw content"
        string content_hash
        string doc_type "standards|runbook|meeting|decision_log|prd|general"
        text window_hashes "JSON {index: sha256} — cheap reclassify"
        text distill_hashes "JSON {index: sha256} — distillation"
        bool stale
        datetime created_at
    }
    ENTITIES {
        int id PK
        int item_id FK
        string kind "decision|document|action|note"
        string summary
        string reasoning
        float confidence
        string author
        string source_ref "source + section"
        text window_text "classifier input window (review context)"
        int window_index
        string status "unverified|verified|disputed|stale"
        string owner
        datetime created_at
        datetime updated_at
    }
    RELATIONSHIPS {
        int id PK
        int from_entity_id FK
        int to_entity_id FK
        string kind "supersedes|depends_on|owns|blocks"
        string source_ref
        float confidence
        datetime created_at
    }
    SOURCES {
        int id PK
        string connector "folder|gdrive|jira|linear|rest"
        string config
        string last_sync_cursor
        datetime last_synced_at
        string last_error
        int error_count
        bool enabled
        string label "internal|public|sensitive|pii"
    }
    CHUNKS {
        int id PK
        int item_id FK "full-document chunk"
        int entity_id FK "entity-summary chunk"
        int section_id FK "→ sections.id"
        string kind "document|entity|distilled|section_summary|doc_summary"
        string source_ref "section"
        string content
        bytes embedding "float32 blob"
        bool is_pii "auto-flagged"
        string pii_categories "JSON"
        int level "heading level"
        string path "section path e.g. Art12 > 12.3"
        datetime created_at
    }
    SECTIONS {
        int id PK
        int item_id FK
        int parent_id FK "self, null=root"
        int level "nesting depth"
        string title "heading text"
        string path "full path"
        string chunk_range "first/last chunk id"
        text summary "extractive summary"
        bytes summary_embedding "float32 blob"
        datetime created_at
    }
    TAGS {
        int id PK
        string name "unique, lowercased"
        string description "optional"
        bytes embedding "float32 blob"
        int count "usage count"
    }
    CHUNK_TAGS {
        int chunk_id FK
        int tag_id FK
    }
    CHUNKS_FTS {
        int rowid "= chunks.id"
        string content "FTS5, kept in sync by triggers"
    }
    JOBS {
        int id PK
        int source_id FK
        string kind "sync|reclassify"
        string status "pending|running|done|failed|cancelled"
        int total
        int processed
        text result
        string error
        datetime started_at
        datetime finished_at
    }
    MERGE_ACTIONS {
        int id PK
        int entity_a_id FK
        int entity_b_id FK
        string status "proposed|merged|dismissed"
        string user
        datetime created_at
    }
    APP_SETTINGS {
        string key PK
        text value "runtime overrides of .env defaults"
        datetime updated_at
    }
    SYSTEM_EVENTS {
        int id PK
        string component
        string level "error|warning|info"
        int source_id FK
        string message
        text detail "redacted"
        datetime created_at
    }
    PROJECTS {
        int id PK
        string name
        bool is_default "user's default query scope"
        datetime created_at
    }
    USERS {
        int id PK
        string email "unique"
        string password_hash "pbkdf2"
        datetime created_at
    }
    VEC_CHUNKS {
        int rowid "virtual vec0 768d cosine (SQLite-only)"
        bytes embedding
    }
    PROJECT_SOURCES {
        int project_id FK
        int source_id FK "many-to-many"
    }
    SOURCES ||--o{ INGESTED_ITEMS : "contains"
    INGESTED_ITEMS ||--o{ ENTITIES : "classified into"
    INGESTED_ITEMS ||--o{ CHUNKS : "chunked"
    INGESTED_ITEMS ||--o{ SECTIONS : "section tree"
    SECTIONS ||--o{ SECTIONS : "parent/children"
    SECTIONS ||--o{ CHUNKS : "contains"
    ENTITIES ||--o{ RELATIONSHIPS : "participates"
    ENTITIES ||--o{ CHUNKS : "summarized into"
    ENTITIES ||--o{ MERGE_ACTIONS : "proposed"
    CHUNKS ||--o| CHUNKS_FTS : "indexed"
    CHUNKS ||--o{ CHUNK_TAGS : "tagged"
    TAGS ||--o{ CHUNK_TAGS : "labels"
    SOURCES ||--o{ JOBS : "processed by"
    PROJECTS ||--o{ PROJECT_SOURCES : "contains"
    SOURCES ||--o{ PROJECT_SOURCES : "belongs to"

    CONTRADICTIONS {
        int id PK "planned, v2 — not in schema yet"
        int claim_a_id FK
        int claim_b_id FK
        string kind
        string evidence
        string status
    }
```

## 6. Container Table

| Container | Responsibility | Tech |
|---|---|---|
| Web UI | Connect sources, project scoping, review queue, PII review, Q&A chat, model settings | React SPA (Vite, TS, 7 tabs) |
| API | All endpoints, orchestration, config | **Rust Axum (shipped)** — FastAPI `backend/` deprecated except `hosting` |
| Entity Store | Entities, provenance, window context, sync state | SQLite local (Rust `rusqlite`) / Postgres hosted (Python legacy) |
| Vector Store | Chunk embeddings + similarity search | sqlite-vec vec0, same file (Rust static); pgvector image for hosted (Python-scan fallback on Postgres) |
| Keyword Store | FTS5 bm25 for hybrid retrieval | SQLite FTS5 (trigger-synced; skipped on Postgres) |
| Classifier | Doc-type detection + entity extraction | Ollama or cloud OpenAI-compatible + rule fallback (Rust `classifier.rs`) |
| Embedder | Chunk embeddings | Ollama or cloud OpenAI-compatible (Rust `embedder.rs`) |
| Answer Engine | Planner → Executor → RRF fusion → graph walk → cited answer | BYO cloud model or Ollama (Rust `answer.rs`/`retrieval.rs`) |
| Retrieval | Vector cosine + FTS5 bm25, who_knows owner/author ranking, `relationships` graph walk | sqlite-vec + FTS5 + SQL (Rust) |
| Job Manager | Background sync/reclassify + cancel | `jobs` table, `MAX_CONCURRENT=2` (Rust `jobs.rs`) |
| Settings Service | Runtime-mutable model config | `app_settings` table + SecretStore (Rust `settings.rs`) |
| Secret Store | Credentials (Jira token, model keys) | OS Keychain via `keyring` + encrypted-file fallback (Rust `secrets.rs`) |

## 7. Security

- Credentials: **OS Keychain** (`keyring`), encrypted-file fallback; never in DB/config/logs.
- Local server binds 127.0.0.1 (hosted binds `0.0.0.0` behind a CORS allowlist).
- Secret values masked in logs, error responses, and API payloads (`***set***`).
- Tests never touch the real keychain (`ANCHOR_SECRETS_NO_KEYRING`).
- **Data labels:** `sources.label` (`public|internal|sensitive|pii`, default
  `internal`); `chunks.is_pii` auto-flagged. Sensitive/PII content skips
  cloud classify/embed/answer, falls back to local rules, and records a
  `system_event`. Share/MCP surface (`GET /qa/public`, `public_only`) only
  sees public sources. Per-user ACL deferred to hosted.
- Every allow/block decision is audited in `system_events`; hosted auth
  issues HS256 JWT when `ANCHOR_AUTH_SECRET` is set, otherwise disabled
  (local stays single-user).

## 8. Key Interfaces (Swap Points)

| Interface | v1 (shipped) | v2+ swap |
|---|---|---|
| `VectorStore` | sqlite-vec | Qdrant / pgvector (hosted) |
| `ModelClient` | Ollama + BYO OpenAI-compatible | Any provider |
| `SecretStore` | Keychain | Cloud secret manager |
| `Connector` | Folder, Drive, Jira, Linear, REST | Slack, Notion, Confluence |
| `AgentAdapter` | MCP sidecar (stdio, read-only) — see [mcp.md](./mcp.md) | MCP HTTP transport / registry / write-back |
| `SettingsService` | DB-backed overrides + keychain secrets | Cloud config service |
| `DB` | SQLite local · Postgres hosted (Python) | Postgres for Rust if hosted is ported |

## 9. Evolution Path

```
v1 (shipped, 1.0.12): local-first app — folder/Drive/Jira/Linear/REST connectors,
    document-aware classification + review queue, cited Q&A (hybrid RRF + vec0 +
    planner/executor + graph walk + distillation + projects + PII gate),
    hierarchical sections, MCP sidecar, Team self-hosted (Docker + offline license),
    Windows/macOS apps
  -> next: website + launch, Drive OAuth, Slack connector, hosted pilot
           (per-user ACL, contradictions, workspace export/delete)
  -> v3: agentic levels (draft, prepare-action-with-approval), enterprise compliance
```

---

*Companion docs: [product-plan.md](./product-plan.md), [packaging.md](./packaging.md), [releasing.md](./releasing.md), [mcp.md](./mcp.md), [design-system.md](./design-system.md)*
