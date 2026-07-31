# AnchorCore — Product Plan (v0.1)

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

Organizations fragment knowledge across Slack, Jira, Confluence, Notion, Google Drive, GitHub, PDFs, and email.

AI assistants fail because:

- Context windows are limited.
- Important decisions get lost.
- Historical reasoning disappears.
- Teams repeat work.
- Institutional knowledge leaves with employees.

## 3. Product Vision

A system that becomes an organization's memory. It understands decisions, ownership, dependencies, timelines, requirements, risks, and historical context — so AI behaves like an employee who has worked at the company for years.

## 4. Initial Wedge — AI PM Operating System

Do **not** start as a generic AI platform. Start narrow.

**Target users:** Product Managers, Product Leaders, Engineering Managers.

**Signature questions:**
- Why did we build this feature?
- Why was this roadmap item delayed?
- What customers requested this?
- What decisions were made, and why?

## 5. How Data Becomes Knowledge

```
Data sources (Documents, Jira, Slack, GitHub)
        |
        v
Extract text + metadata
        |
        v
Identify entities, decisions, requirements, people, systems
        |
        v
Create knowledge objects
        |
        v
Store relationships
```

**Example knowledge object:**

| Field | Value |
|---|---|
| Decision | Security Transfers MVP excludes incoming transfers |
| Reason | Reconciliation complexity |
| Source | PRD v3 |
| Confidence | 82% |

## 6. Avoiding False Facts

AI must not create unquestionable truth. Every knowledge item carries:

- source
- confidence
- timestamp
- author
- verification status

The system detects and flags contradictions (e.g., *"Only EUR supported"* vs. *"USD support added"*).

## 7. Seamless UX

Minimal friction. No manual uploads, no tagging, no second knowledge base to maintain.

**Ideal flow:** user connects existing tools → system continuously learns → the user feels *"the AI already knows."*

## 8. Agentic Progression (after memory)

| Level | Capability |
|---|---|
| 1 | Ask → Answer |
| 2 | Ask → Research → Draft |
| 3 | Ask → Research → Prepare action → Human approval |
| 4 | Autonomous execution |

Examples: weekly product intelligence report, incident investigation, PRD creation, roadmap analysis.

## 9. MVP — Thin Vertical Slice

**Do not build:** billing, enterprise auth, many connectors, complex UI.

**Build:**
1. Upload documents.
2. Index knowledge.
3. Ask questions.
4. Return answers with citations.
5. Extract decisions.

**First connector:** Jira (matches PM workflows).

## 10. Competitive Position

Do not compete with Claude, ChatGPT, or Gemini on intelligence. AnchorCore owns: context, memory, permissions, integrations, organizational history.

## 11. Monetization

| Segment | Price |
|---|---|
| Individual | €15–50 / month |
| Teams | €20–50 / user / month |
| Enterprise | €20k–100k+ / year |

Enterprise value: privacy, security, permissions, compliance, on-prem deployment.

## 12. Go-To-Market

- **First customers:** personal network, PM communities, startup teams.
- **Validation:** 20 conversations, 5 testers, first paid pilot.
- **Marketing:** LinkedIn thought leadership, content on AI context and institutional memory, founder-led outreach.
- **Message:** *"Preserve company knowledge and make every AI model smarter."* (Not "AI database".)

## 13. Employment Conflict Considerations

While employed: no company data, examples, or resources; no fintech-specific clone. Build generic AI knowledge tools with public/synthetic data; discuss the broader problem publicly.

## 14. Long-Term Opportunity

- AI memory layer
- AI knowledge governance
- Context infrastructure for agents
- AI operating system for regulated teams

---

## 15. Milestones (Suggested)

| Phase | Goal | Exit criteria |
|---|---|---|
| 0 — Foundations | Repo, CI, skeleton | App boots; docs render |
| 1 — Vertical slice | Document upload → index → Q&A with citations | Demo works end-to-end |
| 2 — Jira connector | Ingest Jira → knowledge objects → decisions | Answer "why was this delayed?" |
| 3 — Memory core | Provenance, contradiction detection, verification | Trust model demonstrable |
| 4 — Pilot | 5 testers, first paid pilot | Revenue + validation data |

---

*Next step: see [architecture.md](./architecture.md) for the system view.*
