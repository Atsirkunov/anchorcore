# AnchorCore — Architecture Overview (v0.1)

> Target stack (from strategy notes): **Python FastAPI · Next.js · PostgreSQL · Qdrant · Ollama · Cloud LLM APIs**
> This document is a high-level view, not a spec. Keep it in sync as the system evolves.

---

## 1. System Context (C4 level 1)

```
                    +---------------------------+
                    |  Users (PMs / Eng Mgrs)   |
                    +-------------+-------------+
                                  |
                                  | HTTPS
                                  v
                    +---------------------------+
                    |      AnchorCore UI        |  (Next.js)
                    +-------------+-------------+
                                  |
                                  v
                    +---------------------------+
                    |    AnchorCore Platform    |  (API + services)
                    +-------------+-------------+
                                  |
              +-------------------+-------------------+
              |                   |                   |
              v                   v                   v
   +------------+        +------------+        +------------+
   | Jira       |        | Slack      |        | Notion     |
   | Docs/GitHub|        | Drive etc. |        | Cloud LLMs |
   +------------+        +------------+        +------------+
        external sources                          model providers
```

AnchorCore is the memory layer between an organization's tools and any AI model.

## 2. Container View (C4 level 2)

```
+-----------------------------+        +----------------------------+
|   UI (Next.js)              |        |   API (FastAPI)            |
|  - Chat / Q&A               |<------>|  - Auth / permissions      |
|  - Sources & connectors mgmt|        |  - Ingestion orchestration |
|  - Knowledge explorer       |        |  - Q&A endpoint            |
+-----------------------------+        |  - Extraction jobs         |
                                       +-------------+--------------+
                                                     |
                    +--------------------------------+--------------------------------+
                    |                                 |                                |
                    v                                 v                                v
        +-------------------------+     +-------------------------+     +-------------------------+
        |  Connectors             |     |  Extraction / Ingestion |     |  Memory Store           |
        |  Jira, Slack, Notion,   |     |  Parsers -> normalizer  |     |  PostgreSQL (facts,     |
        |  Drive, GitHub, uploads |     |  LLM extraction of      |     |    relations, sources,  |
        |                         |     |  entities / decisions   |     |    provenance)          |
        +-------------------------+     +-------------------------+     +-------------------------+
                                                                |
                                                                v
                                                     +-------------------------+
                                                     |  Qdrant (vector index)  |
                                                     |  embeddings for semantic |
                                                     |  retrieval               |
                                                     +-------------------------+
                                                                |
                                                                v
                                                     +-------------------------+
                                                     |  Model layer            |
                                                     |  Ollama (local) + cloud |
                                                     |  LLM APIs (OpenAI etc.) |
                                                     +-------------------------+
```

## 3. Data Flow — Knowledge Pipeline

```
[Source] -> Extract text + metadata -> Normalize -> Identify entities/decisions/requirements
          -> Create knowledge objects -> Store relationships (PG) + embeddings (Qdrant)
          -> Index ready for retrieval
```

**Retrieval (Q&A):**
```
Question -> Embed query -> Hybrid search (vector + structured + relations)
        -> Gather context (permission-filtered) -> LLM answer -> citations from sources
```

## 4. Proposed Tech Stack

| Layer | Choice | Notes |
|---|---|---|
| UI | Next.js (TypeScript) | Chat, connectors, knowledge explorer |
| API | Python FastAPI | Async, jobs, SSE streaming |
| Primary store | PostgreSQL | Facts, decisions, sources, permissions, provenance |
| Vector store | Qdrant | Semantic search + hybrid retrieval |
| Extraction | LLM-driven (Ollama local / cloud APIs) | Entity + decision extraction |
| Models | Ollama (local, dev) + cloud LLM APIs | Provider-agnostic abstraction |
| Connectors | One at a time (Jira first) | Webhooks + polling |
| Infra | Docker Compose (dev), later K8s | On-prem path for enterprise |

## 5. Data Model Sketch (Core Entities)

- **Source** — a connected tool + credential reference.
- **Document** — normalized unit of ingested content (raw text + metadata).
- **Entity** — person, system, customer, feature, requirement.
- **KnowledgeObject** — decision, requirement, risk, timeline item.
  - fields: type, summary, reasoning, confidence, timestamp, author, verification status, source ref.
- **Relationship** — links objects/entities (e.g., *delays*, *depends on*, *supersedes*).
- **Contradiction** — detected conflicts between knowledge objects (with claim refs).
- **Workspace / User / Permission** — access control to knowledge.

## 6. Provenance & Trust (Non-Negotiable)

Every knowledge item carries: source, confidence, timestamp, author, verification status.
Contradiction detection flags stale/conflicting claims (e.g., currency support change) — surfaced in the UI, never silently resolved.

## 7. Security & Privacy

- Permission-aware retrieval: answers only include knowledge the user may see.
- No credentials stored in plaintext; secret manager for connector tokens.
- Enterprise: on-prem deployment, audit log, compliance posture.
- Model provider choice per workspace (local via Ollama when required).

## 8. Evolution Path

```
Phase 1: upload + Q&A (citations)
  -> Phase 2: Jira connector
  -> Phase 3: memory core (provenance, contradictions, verification)
  -> Phase 4: agentic levels (draft, prepare action w/ approval, autonomous)
```

---

*Companion doc: [product-plan.md](./product-plan.md)*
