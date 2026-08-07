# AnchorCore — Architecture Overview (v1.0)

> Locked through structured product/architecture drilldown (13 decisions).
> Stack: **FastAPI · React (Vite) · SQLite + sqlite-vec · Ollama · BYO cloud LLM APIs**

---

## 1. Runtime Model: Local-First (Plex-Style)

One local process = the entire product. No Docker, no sidecar services, no setup.

## 2. System Diagram

```mermaid
flowchart TB
    subgraph laptop["User's Laptop (Windows/macOS)"]
        direction TB
        browser["Browser UI<br/>(React SPA, localhost)"]
        api["AnchorCore App<br/>FastAPI (single process)"]
        api --- connectors["Connectors<br/>Folder watch · Jira poll"]
        api --- pipeline["Ingestion Pipeline<br/>type detect → classify → entities"]
        api --- rag["Answer Engine<br/>RRF hybrid RAG + citations"]
        api --- review["Review API<br/>low-confidence · duplicates"]
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

    jira["Jira Cloud API"]
    folder["Local folder<br/>(watched)"]
    cloud["BYO LLM API<br/>(OpenAI-compatible)"]

    connectors --- folder
    connectors --- jira
    pipeline --- cloud
    rag --- cloud
```

## 3. Data Flow — Ingestion

```mermaid
flowchart LR
    watcher["Folder watcher<br/>fs events + hourly scan"] --> extract["Extract text"]
    jira["Jira poll<br/>15 min, incremental"] --> extract
    extract --> hash["Hash dedup"]
    hash --> type["Document-type pre-pass<br/>(1 cheap call, cached)"]
    type --> classify["Classify windows<br/>(local Ollama or cloud, parallel)"]
    classify --> entities["Entities + window context<br/>(SQLite)"]
    classify --> embed["Embed chunks<br/>(local or cloud)"]
    embed --> vec["sqlite-vec index"]
    classify --> fts["FTS5 keyword index"]
    entities --> review["Review UI<br/>context panel · confirm / merge / reclassify"]
```

**Deletion policy:** sources removed → marked `stale`; knowledge is never auto-deleted.

## 4. Data Flow — Q&A

```mermaid
sequenceDiagram
    participant U as User (browser)
    participant A as AnchorCore App
    participant V as sqlite-vec
    participant F as FTS5 (chunks_fts)
    participant O as Ollama
    participant M as BYO LLM API

    U->>A: "What was decided about X, and why?" (+ chat history)
    A->>A: rewrite follow-up into standalone query (if history)
    A->>O: embed query
    O-->>A: query vector
    A->>V: top-k similarity
    A->>F: FTS5 keyword (bm25)
    A->>A: RRF fusion + age decay + diversity cap + context expansion
    A->>A: graph walk (B32) — connected entities join context + citations
    A->>M: answer prompt w/ sections (+ conversation)
    M-->>A: answer + citations
    A-->>U: answer + clickable source chips
```

## 4b. Retrieval Design (informed by Cerebras' knowledge base)

Cerebras published how their internal KB serves 15k queries/day (2026-07).
Their design validated our hybrid approach and added four concrete techniques
we adopted (B12.1):

1. **Multi-scorer fusion, no single trusted scorer** — they run full-text,
   embeddings, and IDF-scored retrievers in parallel and fuse the ranked
   lists. We fuse FTS5 (bm25) + cosine via **reciprocal rank fusion (RRF)**:
   `score = Σ weight / (60 + rank)`. Consensus across scorers beats a single
   strong vote; no score normalization needed.
2. **Per-file diversity cap** — a document that matches broadly must not
   monopolize the top-k. After fusion, cap results per file (default 3);
   relaxed for single-file corpora so a big document's sections can surface.
3. **Age decay** — "Slack answers expire"; when relevance is otherwise equal,
   the newer hit wins. A recency multiplier is applied in fusion.
4. **Context expansion** — once winners are picked, pull the neighboring
   sections (heading, preconditions, caveats) that chunking split apart, so
   the LLM sees a complete section instead of a lonely paragraph.
5. **Near-duplicate chunk dedupe** — TOC entries duplicating body sections
   keep only the best-scoring copy.

Follow-up questions reuse retrieval with a **query-rewrite pass**: history is
sent with the question, a cheap LLM call rewrites it standalone, and the
conversation is included in generation.

**Graph-based retrieval (B32):** after RRF picks the winning entities, the
pipeline walks `relationships` 1–2 hops (`supersedes` > `depends_on` > `owns` >
`blocks` > `related`, hop decay, fan-out cap) and pulls connected entities'
summaries/chunks into the answer context as `[related]` sections + citations.
This answers "what supersedes this?" / "what depends on this decision?" that
lexical+vector fusion alone misses, because the connected knowledge doesn't
need to co-occur in the question's words. Pure SQL — no model calls.

Further learnings parked in the backlog: scoped search via *projects* (bundles
of sources with a per-user default — "search everything everywhere" stops
being useful at scale), *who_knows* expertise queries, planner→executor→
synthesis query architecture, and distillation of raw content into
question/summary/resolution fields before embedding.

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
        string connector "folder|jira|linear|upload"
        string config
        string last_sync_cursor
        datetime last_synced_at
        string last_error
        int error_count
        bool enabled
    }
    CHUNKS {
        int id PK
        int item_id FK "full-document chunk"
        int entity_id FK "entity-summary chunk"
        string source_ref "section"
        string content
        bytes embedding "float32 blob"
        datetime created_at
    }
    CHUNKS_FTS {
        int rowid "= chunks.id"
        string content "FTS5, kept in sync by triggers"
    }
    JOBS {
        int id PK
        int source_id FK
        string kind "sync|reclassify"
        string status "running|done|failed|cancelled"
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
    SOURCES ||--o{ INGESTED_ITEMS : "contains"
    INGESTED_ITEMS ||--o{ ENTITIES : "classified into"
    INGESTED_ITEMS ||--o{ CHUNKS : "chunked"
    ENTITIES ||--o{ RELATIONSHIPS : "participates"
    ENTITIES ||--o{ CHUNKS : "summarized into"
    ENTITIES ||--o{ MERGE_ACTIONS : "proposed"
    CHUNKS ||--o| CHUNKS_FTS : "indexed"
    SOURCES ||--o{ JOBS : "processed by"

    CONTRADICTIONS {
        int id PK
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
| Web UI | Connect sources, review queue, duplicate proposals, Q&A chat, model settings | React SPA (Vite, TS) |
| API | All endpoints, orchestration, config | FastAPI |
| Entity Store | Entities, provenance, window context, sync state | SQLite + SQLAlchemy |
| Vector Store | Chunk embeddings + similarity search | sqlite-vec (same SQLite file) |
| Keyword Store | FTS5 bm25 for hybrid retrieval | SQLite FTS5 (`chunks_fts`, trigger-synced) |
| Classifier | Doc-type detection + entity extraction | Ollama or cloud OpenAI-compatible + rule fallback |
| Embedder | Chunk embeddings | Ollama or cloud OpenAI-compatible |
| Answer Engine | RRF hybrid RAG + citations + follow-up rewrite | BYO cloud model or Ollama |
| Job Manager | Background sync/reclassify + cancel | asyncio tasks + `jobs` table |
| Settings Service | Runtime-mutable model config | `app_settings` table + SecretStore |
| Secret Store | Credentials (Jira token, model keys) | OS Keychain via `keyring` + encrypted-file fallback |

## 7. Security

- Credentials: **OS Keychain** (`keyring`), encrypted-file fallback; never in DB/config/logs.
- Local-only server binds 127.0.0.1.
- Secret values masked in logs, error responses, and API payloads (`***set***`).
- `SecretStore` interface maps to cloud secret managers in hosted v2.
- Tests never touch the real keychain (`ANCHOR_SECRETS_NO_KEYRING`).
- **Data labels (B30, planned)**: sources carry a label (`public|internal|sensitive|pii`);
  each model provider declares a trust tier (`local` vs user-confirmed `cloud`);
  the pipeline gates routing by label×tier (PII → local-only, logged), and
  sharing/answers/MCP exclude non-`public` content outside authorized sessions.
  Every allow/block decision is audited in `system_events`.

## 8. Key Interfaces (Swap Points)

| Interface | v1 | v2+ swap |
|---|---|---|
| `VectorStore` | sqlite-vec | Qdrant / pgvector (hosted) |
| `ModelClient` | Ollama + BYO OpenAI-compatible | Any provider |
| `SecretStore` | Keychain | Cloud secret manager |
| `Connector` | Folder, Jira | Linear, Slack, Notion, Google Drive (B28) |
| `AgentAdapter` | MCP server (stdio + HTTP) — see [mcp.md](./mcp.md) | MCP registry / deeper tool set |
| `SettingsService` | DB-backed overrides + keychain secrets | Cloud config service |
| `DB` | SQLite | PostgreSQL (hosted migration) |

## 9. Evolution Path

```
v1: local-first, folder + Jira, document-aware classification + review + cited Q&A
  -> v1.5: Linear/Drive connectors, Windows exe + macOS app, MCP access
  -> v2: team sharing, hosted option, Slack, contradictions, bundled models
  -> v3: agentic levels (draft, prepare-action-with-approval), enterprise compliance
```

---

*Companion docs: [product-plan.md](./product-plan.md), [packaging.md](./packaging.md), [releasing.md](./releasing.md), [mcp.md](./mcp.md)*
