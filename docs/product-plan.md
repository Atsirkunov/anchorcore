# AnchorCore — Product Plan (v1.0)

> **Working thesis:** The winning layer of the AI stack is not another model — it is the system that gives any AI model the right context, at the right time, with trust and provenance.
>
> **Positioning:** *"Connect your knowledge to any AI model."*
> **Category:** AI memory layer / knowledge operating system / context infrastructure for agents.

---

## 1. Core Thesis

AI models are the reasoning engine. AnchorCore is the organizational memory:

- The AI model = a smart new employee.
- AnchorCore = the company's memory, history, and filing system.

Future AI advantage comes less from bigger models and more from giving models the right context, at the right time, with trust and provenance.

## 2. Problem

Organizations fragment knowledge across Slack, Jira, Confluence, Notion, Google Drive, GitHub, PDFs, and email. AI assistants fail because: context windows are limited, important decisions get lost, historical reasoning disappears, teams repeat work, and institutional knowledge leaves with employees.

## 3. Product Vision

A system that becomes an organization's memory — understanding decisions, ownership, dependencies, timelines, requirements, risks, and historical context, so AI behaves like an employee who has worked at the company for years.

## 4. Customer (v1)

**Buyer:** self-serve knowledge workers — PMs at mid-sized companies, solo entrepreneurs, local enthusiasts. No sales team, no procurement cycle, €15–50/mo willingness.

**Moment of wow (60-second demo):** connect sources → ask *"what was decided about X, and why?"* → get a cited answer showing exactly where the knowledge came from.

## 5. Scope — Locked v1 Decisions

| Area | Decision |
|---|---|
| Runtime | Local-first desktop app (Plex-style local server + browser UI), macOS first |
| Connectors | Local folder watch + Jira (v1); Linear fast-follow; Slack v2 |
| Model tiering | Ollama (local, 3B) for classification; BYO big models via API for Q&A/planning |
| Model access | Bring-your-own-key (v1); bundled access v2–3 |
| Trust layer | Sources + confidence + review UI (v1); contradictions & verification workflows (v2) |
| Retrieval | SQLite + sqlite-vec, local embeddings (`nomic-embed-text`), swappable VectorStore |
| Sync | 15-min Jira incremental polls; folder watcher + hourly scan fallback; hash dedup; stale-not-delete |
| Data model | Uniform entity graph (entities + typed relationships + provenance); contradictions as external layer |
| Answers | Single-pass RAG, section-level citations, cheap default BYO model |
| Identity | Hash dedup in-source; cross-source duplicate proposals for manual merge |
| Billing | Free during validation; flat license at pilot; billing machinery deferred to hosted v2+ |
| Security | OS keychain for credentials; secrets never in DB/config/logs |

## 6. How Data Becomes Knowledge

```
Sources (folder, Jira, Linear, Slack later)
    -> Extract text + metadata (incremental, deduped by hash)
    -> Classify into entity kinds (decision | document | action | note)
    -> Attach entities (person, system, feature, customer) + relationships
    -> Store: SQLite entities/relationships + sqlite-vec embeddings
    -> Review: low-confidence items + cross-source duplicates proposed to user
```

**Example entity:**

| Field | Value |
|---|---|
| Type | decision |
| Summary | Security Transfers MVP excludes incoming transfers |
| Reasoning | Reconciliation complexity |
| Source | PRD v3 (section 4.2) |
| Confidence | 82% |
| Owner | (user-assigned) |

## 7. Trust Model (v1 vs v2)

**v1 — "show your work":**
- Every entity carries source, confidence, timestamp, author, verification status.
- Every answer cites its sources as clickable section-level references.
- Review UI: user confirms/reclassifies low-confidence items (recorded with user as author).
- Duplicate proposals: user merges (never auto-merge).

**v2 (deferred):** contradiction detection (claim graph + pairwise conflict scanning), human verification workflows (verified/disputed states).

## 8. Agentic Progression (after memory)

| Level | Capability |
|---|---|
| 1 | Ask → Answer (v1) |
| 2 | Ask → Research → Draft |
| 3 | Ask → Research → Prepare action → Human approval |
| 4 | Autonomous execution |

## 9. Competitive Position

Do not compete with Claude, ChatGPT, or Gemini on intelligence. AnchorCore owns: context, memory, permissions, integrations, organizational history — local-first, private by default.

## 10. Monetization Path

| Phase | Price |
|---|---|
| Validation (v1, now) | Free — gather feedback |
| Pilot | Flat license, e.g. €25–30/mo or one-time €150–200 |
| v2–3 (hosted) | Individual €15–50/mo; Teams €20–50/user/mo; Enterprise €20k–100k+/yr |

## 11. V2 Features Worthy of Charging For

Candidates, in rough order of revenue pull (all build on the v1 memory core):

1. **Team memory + sharing** — shared workspace, permissions; the natural Teams tier trigger.
2. **Hosted sync/summary** — "the AI already knows" without running your own machine; justifies SaaS pricing.
3. **Bundled model access** — non-technical users who won't paste keys; margin on inference.
4. **Contradiction detection & verification workflows** — the enterprise trust story (audit-grade provenance).
5. **Agentic levels 2–3** — research drafts, action preparation with human approval; the "productivity" premium.
6. **Slack connector** — decision-rich source once OAuth complexity is solved.
7. **Enterprise compliance** — SSO, audit logs, on-prem deployment (€20k+ deals).

## 12. Go-To-Market (v1)

- **First customers:** personal network, PM communities, startup teams.
- **Validation:** 20 conversations, 5 testers, first paid pilot.
- **Marketing:** LinkedIn thought leadership on AI context and institutional memory; founder-led outreach.
- **Message:** *"Preserve company knowledge and make every AI model smarter."* (Not "AI database".)

## 13. Milestones

| Phase | Goal | Exit criteria |
|---|---|---|
| 1 — Foundation | Skeleton: backend, UI, DB, Ollama integration | App boots locally |
| 2 — Folder ingest | Watch folder → classify → review UI | "Show your work" demo works |
| 3 — Jira connector | Poll Jira → entities + relationships | Answer "why was this delayed?" |
| 4 — Q&A | RAG answers with section citations | 60-second wow demo end-to-end |
| 5 — Validation | 20 conversations, 5 testers | Feedback + first paid pilot |
| 6 — Packaging | macOS executable, Ollama-first | Distributable .dmg |

---

## 13. Backlog

Priorities: P1 = testers hit it during validation, P2 = quality/trust, P3 = later.

### B1. Error transparency + downloadable logs (P1)
**Problem:** today errors surface as raw "Internal Server Error" with no context; logs only exist in the console/file on the machine. Test users can't help debug.

**Scope:**
- Structured error records (component, source, timestamp, message) surfaced in a "System" tab, not just a raw log tail
- Context on errors: which source/connector/step failed, last sync status, model availability
- **Download log button** in the app serving `data/anchorcore.log` (current file; rotated files via UI list)
- Secrets never in logs (audit existing paths — connector errors, config dumps)
- Frontend surfaces the specific error text (already partially done — make consistent across all actions)

**DoD:** a tester hits a failure and can export a log file + error context in two clicks.

### B2. Progress feedback on every action (P1)
**Problem:** sync/reclassify are synchronous — long operations look frozen, large sources can time out, and there's no "is it done?" signal.

**Scope:**
- Busy states on all buttons (spinner/disabled) — small, immediate
- **Background job model** for sync/reclassify: job starts immediately, UI polls job status (running / done / failed, processed X of Y, entities produced)
- Job history (last N jobs per source) — doubles as an error surface
- Progress shown inline in Sources tab

**DoD:** reclassify of a large source shows live progress and completion, never a browser timeout.

### B3. Entity dispute tracking (P2)
**Problem:** "dispute" currently just flips a status flag — no record of who disputed, when, or why. The trust story needs an audit trail.

**Scope:**
- `disputes` table: entity_id, timestamp, reason (optional), user
- Dispute counter on the entity + activity timeline in the UI
- Disputed entities excluded from Q&A context by default (configurable) — the "never present contested facts as truth" behavior
- (v2: full verification workflow per product plan)

**DoD:** dispute an entity → counter increments, reason stored, answers stop citing it.

### B4. LLM configuration in app (P1)
**Problem:** model settings live in `backend/.env` — restart required, invisible, blocked testers ("how do I connect my key?").

**Scope:**
- Settings tab: provider presets (Ollama local / OpenAI-compatible cloud / custom base URL), classifier model, embed model, answer model + API key
- BYO keys via existing `SecretStore` (keychain), never in DB/config/logs
- Runtime-mutable settings: DB-backed `app_settings` overriding env defaults; `Settings` becomes a runtime service, not an import-time singleton
- "Test connection" button per provider (verifies key/model reachability)
- Health banner reads live provider; no restart needed

**DoD:** a tester connects their own model key from the UI in under 60s, no config file, no restart.

### B5. Full-document chunking (P1 — discovered)
**Problem:** today only *entity summaries* are chunked and embedded, and the classifier input is truncated at ~12k chars. A big PDF (like the current test file) loses most of its content — most of the document is never retrievable.

**Scope:**
- Chunk the full document text (section-aware, ~800 tokens with overlap — config already exists)
- Embed all chunks, tag each with source ref + entity links
- Classifier input: process long documents in sections instead of one truncated call
- QA retrieval uses full-document chunks (citations point to sections)

**DoD:** a 300-page PDF is fully indexed; questions about content in page 250 return cited answers.

### B6. Embedding backfill job (P1 — discovered)
**Problem:** when embedding fails (Ollama down), chunks are stored unembedded — and nothing ever retries them unless the file changes. The memory silently stays keyword-only.

**Scope:**
- Periodic (and on-startup) job: embed all chunks with missing embeddings
- Health component: "N chunks pending embedding" so it's visible, not silent

**DoD:** after Ollama comes back, all pending chunks get embedded without user action, and health reports the catch-up.

### B7. Source configuration editing in UI (P2)
**Problem:** editing a folder path or Jira credentials requires delete + recreate.

**Scope:** `PUT /sources/{id}` (config, name, enabled), edit form in Sources tab; secret fields stay keychain-backed.

### B8. First-run wizard (P2)
**Scope:** on first launch: check Ollama → offer install/pull instructions, model selection, quick folder connect, sample question. Turns the 60-second wow into the onboarding path.

### B9. Document type coverage (P2)
**Scope:** add `.docx`/`.pptx`/`.odt` extraction (small deps); revisit after real tester file types are known — the watched folder currently only ingests a subset of formats.

### B10. Model configuration via UI — superseded by B4. (see B4)

---

*Companion docs: [architecture.md](./architecture.md), [packaging.md](./packaging.md)*
