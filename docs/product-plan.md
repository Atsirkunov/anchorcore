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

*Companion docs: [architecture.md](./architecture.md), [packaging.md](./packaging.md)*
