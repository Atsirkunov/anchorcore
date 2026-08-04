# AnchorCore — Architecture Overview (v1.0)

> Locked through structured product/architecture drilldown (13 decisions).
> Stack: **FastAPI · React (Vite) · SQLite + sqlite-vec · Ollama · BYO cloud LLM APIs**

---

## 1. Runtime Model: Local-First (Plex-Style)

One local process = the entire product. No Docker, no sidecar services, no setup.

## 2. System Diagram

```mermaid
flowchart TB
    subgraph laptop["User's Laptop (macOS)"]
        direction TB
        browser["Browser UI<br/>(React SPA, localhost)"]
        api["AnchorCore App<br/>FastAPI (single process)"]
        api --- connectors["Connectors<br/>Folder watch · Jira poll"]
        api --- pipeline["Ingestion Pipeline<br/>Extract → classify → entities"]
        api --- rag["Answer Engine<br/>RAG + section citations"]
        api --- review["Review API<br/>low-confidence · duplicates"]
        db[("Entity Graph<br/>SQLite + sqlite-vec")]
        secrets["Secrets<br/>macOS Keychain"]
        api --- db
        api --- secrets
        browser --- api
        ollama["Ollama<br/>3B classifier<br/>nomic-embed-text"]
        api --- ollama
    end

    jira["Jira Cloud API"]
    folder["Local folder<br/>(watched)"]
    cloud["BYO LLM API<br/>(OpenAI-compatible)"]

    connectors --- folder
    connectors --- jira
    rag --- cloud
```

## 3. Data Flow — Ingestion

```mermaid
flowchart LR
    watcher["Folder watcher<br/>fs events + hourly scan"] --> extract["Extract text"]
    jira["Jira poll<br/>15 min, incremental"] --> extract
    extract --> hash["Hash dedup"]
    hash --> classify["Classify<br/>(Ollama 3B)"]
    classify --> entities["Entities + relationships<br/>(SQLite)"]
    classify --> embed["Embed chunks<br/>(nomic-embed-text)"]
    embed --> vec["sqlite-vec index"]
    entities --> review["Review UI<br/>confirm / merge / reclassify"]
```

**Deletion policy:** sources removed → marked `stale`; knowledge is never auto-deleted.

## 4. Data Flow — Q&A

```mermaid
sequenceDiagram
    participant U as User (browser)
    participant A as AnchorCore App
    participant V as sqlite-vec
    participant O as Ollama
    participant M as BYO LLM API

    U->>A: "What was decided about X, and why?"
    A->>O: embed question
    O-->>A: query vector
    A->>V: top-k similarity
    V-->>A: sections (tagged w/ source refs)
    A->>M: answer prompt w/ retrieved sections
    M-->>A: answer + citations
    A-->>U: answer + clickable source chips
```

## 5. Data Model — Uniform Entity Graph

Everything is an entity. Contradictions stay external to the baseline object.

```mermaid
erDiagram
    ENTITIES {
        int id PK
        string type "decision|document|action|note|person|system|feature|customer"
        string summary
        string reasoning
        float confidence
        string author
        string source_ref "source + section"
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
        string config_ref
        string last_sync_cursor
    }
    CHUNKS {
        int id PK
        int entity_id FK
        string source_ref "section"
        string content
        vector embedding
    }
    MERGE_ACTIONS {
        int id PK
        int entity_a_id FK
        int entity_b_id FK
        string status "proposed|merged|dismissed"
        string user
        datetime created_at
    }
    ENTITIES ||--o{ RELATIONSHIPS : "participates"
    ENTITIES ||--o{ CHUNKS : "chunked"
    ENTITIES ||--o{ MERGE_ACTIONS : "proposed"

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
| Web UI | Connect sources, review queue, duplicate proposals, Q&A chat | React SPA (Vite, TS) |
| API | All endpoints, orchestration, config | FastAPI |
| Entity Store | Entities, relationships, provenance, sync state | SQLite + SQLAlchemy |
| Vector Store | Chunk embeddings + similarity search | sqlite-vec (same SQLite file) |
| Classifier | Entity kind extraction | Ollama 3B + rule fallback |
| Embedder | Chunk embeddings | Ollama `nomic-embed-text` |
| Answer Engine | RAG composition + citations | BYO cloud model (OpenAI-compatible) |
| Secret Store | Credentials (Jira token, model keys) | macOS Keychain via `keyring` |

## 7. Security

- Credentials: **macOS Keychain** (`keyring`), encrypted-file fallback; never in DB/config/logs.
- Local-only server binds 127.0.0.1.
- Secret values masked in logs and error responses.
- `SecretStore` interface maps to cloud secret managers in hosted v2.

## 8. Key Interfaces (Swap Points)

| Interface | v1 | v2+ swap |
|---|---|---|
| `VectorStore` | sqlite-vec | Qdrant / pgvector (hosted) |
| `ModelClient` | Ollama + BYO OpenAI-compatible | Any provider |
| `SecretStore` | Keychain | Cloud secret manager |
| `Connector` | Folder, Jira | Linear, Slack, Notion, Drive |
| `AgentAdapter` | MCP server (stdio + HTTP) — see [mcp.md](./mcp.md) | MCP registry / deeper tool set |
| `DB` | SQLite | PostgreSQL (hosted migration) |

## 9. Evolution Path

```
v1: local-first, folder + Jira, classification + review + cited Q&A
  -> v1.5: Linear connector, packaging (.dmg)
  -> v2: team sharing, hosted option, Slack, contradictions, bundled models
  -> v3: agentic levels (draft, prepare-action-with-approval), enterprise compliance
```

---

*Companion docs: [product-plan.md](./product-plan.md), [packaging.md](./packaging.md)*
