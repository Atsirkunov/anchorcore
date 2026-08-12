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

### B1. Error transparency + downloadable logs — DONE (commit: add B1 troubleshooting pass)
**Problem:** today errors surface as raw "Internal Server Error" with no context; logs only exist in the console/file on the machine. Test users can't help debug.

**Done:**
- Structured error records (`system_events` table: component, source, level, message, detail, timestamp) recorded by pipeline/scheduler/embedder/QA failure paths, surfaced in a **System tab** (filter by component/level, source name)
- `/system/status`: model availability, Ollama reachability, answer provider, scheduler tasks, pending-embedding count, failing sources
- **Log download**: `/system/logs` lists `anchorcore.log` + rotated files; download endpoint (sanitized, no path traversal)
- **Secrets never in logs/UI**: `RedactingFormatter` on console + file handlers, redaction applied to stored error text and `source.last_error`; `GET /sources/{id}/config` masks tokens (was leaking the Jira token in plaintext)
- Frontend surfaces specific error text consistently (fetch `detail` parsed in `api.ts`)

**DoD:** a tester hits a failure and can export a log file + error context in two clicks. — met via System tab → Download log.

### B2. Progress feedback on every action (P1) — DONE
**Problem:** sync/reclassify are synchronous — long operations look frozen, large sources can time out, and there's no "is it done?" signal.

**Done:** background job model (202 + polling + history + progress bars in Sources tab, job history in System tab) landed with B13; busy/disabled states + per-item indicators on review/merge/verify/dispute actions, QA spinner + "Thinking…" state, and a running-jobs summary badge in the header (polls `/sources/jobs/running`) landed in the B2 remainder pass.

**DoD:** reclassify of a large source shows live progress and completion, never a browser timeout. — met.

### B3. Entity dispute tracking (P2) — DONE
**Problem:** "dispute" currently just flips a status flag — no record of who disputed, when, or why. The trust story needs an audit trail.

**Done:**
- `disputes` table (entity_id, reason, user, created_at) + `entities.dispute_count` — migration `b3a0c1`
- `POST /entities/{id}/dispute` records who/when/why, increments the counter, marks the entity disputed; `GET /entities/{id}/disputes` returns the audit trail (newest first)
- Review tab "Dispute" now asks for a reason inline and shows a `⚑ disputed ×N` badge
- **Disputed entities excluded from Q&A retrieval by default** — `AnswerEngine._status_ok` filters `stale` always and `disputed` unless `ANCHOR_QA_EXCLUDE_DISPUTED=false`; applied uniformly across vector, FTS, who_knows, fallback and graph expansion
- Tests (`tests/test_disputes.py`): audit trail + counter increments, and the DoD — the entity is cited before the dispute and stops being cited after

**DoD:** dispute an entity → counter increments, reason stored, answers stop citing it. — met.

### B4. LLM configuration in app (P1) — DONE
**Problem:** model settings live in `backend/.env` — restart required, invisible, blocked testers ("how do I connect my key?").

**Done:**
- **Settings tab** in the UI: provider presets (Ollama local / OpenAI-compatible cloud / custom), classifier model, embed model, answer model + API key
- **BYO keys via `SecretStore`**: `answer_api_key` stored under `app:` keys in the keychain (encrypted-file fallback), never in DB/config/logs; responses mask it as `***set***`
- **Runtime-mutable settings**: new `app_settings` table (migration `b4a00c1`) + `SettingsService` — env `.env` stays the default layer, DB overrides win, secrets resolve from the SecretStore. `Classifier`/`Embedder`/`AnswerEngine`/`status` probes now read through the service at call time → **no restart needed**
- **"Test connection"** per provider: `/settings/test-connection` verifies Ollama reachability + model presence, and answer-provider reachability (key required for non-local)
- Health banner + System tab read live provider values
- Tests: CRUD + persistence, unknown-key 422, secret masking + keychain storage, clear→env fallback, test-connection failure paths, runtime values in `/system/status`

**DoD:** a tester connects their own model key from the UI in under 60s, no config file, no restart. — met.

### B5. Full-document chunking (P1 — discovered) — DONE (commit: 4e9cdcc)
**Problem:** today only *entity summaries* are chunked and embedded, and the classifier input is truncated at ~12k chars. A big PDF (like the current test file) loses most of its content — most of the document is never retrievable.

**Done:**
- Chunk the full document text (section-aware, ~800 tokens with overlap — config exists)
- Embed all chunks, tag each with source ref + entity links
- Classifier input: process long documents in windows instead of one truncated call (later deepened by B11 parallel + B26 16k windows)
- QA retrieval uses full-document chunks (citations point to sections)

**DoD:** a 300-page PDF is fully indexed; questions about content in page 250 return cited answers. — met (live rulebook probes).

### B6. Embedding backfill job — DONE (commit: add B1 troubleshooting pass)
**Problem:** when embedding fails (Ollama down), chunks are stored unembedded — and nothing ever retries them unless the file changes. The memory silently stays keyword-only.

**Status:** backfill loop (on startup + every 15 min) landed with B13; visibility done in the troubleshooting pass — `pending_embeddings` count in `/health` + `/system/status`, "Embedding backfill" card in the System tab.

**DoD:** after Ollama comes back, all pending chunks get embedded without user action, and health reports the catch-up.

### B7. Source configuration editing in UI (P2) — DONE
**Problem:** editing a folder path or Jira credentials requires delete + recreate.

**Done:**
- `PUT /sources/{id}` (name, enabled, config) — partial updates; unknown/empty-name → 422; scheduler reloads watchers after a change
- Edit form per source in the Sources tab (folder path / Jira fields pre-filled from the masked config); secret fields stay keychain-backed — the `***set***` placeholder keeps the stored value, an explicit empty string clears it
- **Fixed a latent SecretStore fallback bug**: the fallback was a single encrypted blob, so `delete(key)` wiped *every* stored secret; it is now per-key files (`secrets.enc.<sha256(key)>`)
- Tests (`tests/test_source_edit.py`): name/enabled edits, config change + re-sync against the new path, secret keep/replace/clear

**DoD:** editing a folder path or Jira credentials no longer requires delete + recreate. — met.

### B8. First-run wizard (P2) — DONE
**Done:**
- `GET /system/onboarding` — the wizard trigger: `needs_wizard` (true with zero sources), Ollama reachability + missing models, answer-provider state, and the bundled `sample/` corpus path when resolvable (None when frozen)
- `OnboardingWizard` overlay in the UI (auto-shown on first launch, dismissible): step 1 setup checks (Ollama install/pull instructions per platform), step 2 connect a folder or the sample corpus with live sync progress, step 3 ask a question and see the cited answer
- Tests: onboarding endpoint shape + source-count delta

**DoD:** on first launch a new user is guided from "install Ollama" to a cited answer in a few clicks, no config file. — met (dev; frozen apps hide the sample button).

### B9. Document type coverage (P2)
**Scope:** add `.docx`/`.pptx`/`.odt` extraction (small deps); revisit after real tester file types are known — the watched folder currently only ingests a subset of formats.

### B11. Parallel classification throughput (P2 — discovered) — DONE
**Problem:** windowed classification runs sequentially — a 237-page PDF took ~15 min (160+ serial LLM calls). Hardware is fine; the pipeline serializes.

**Done:**
- Bounded concurrency in the pipeline: `asyncio.Semaphore(N)` around classifier calls (N = `ANCHOR_CLASSIFIER_CONCURRENCY`, default 4) — windows are classified with `asyncio.gather`, results concatenated in order
- Ollama tuning documented (README + `.env.example`): `OLLAMA_NUM_PARALLEL=4` (match app concurrency), `OLLAMA_CONTEXT_LENGTH=8192` (windows need ~2.5k tokens; 32k default wastes KV cache), `OLLAMA_KEEP_ALIVE=30m` (avoid model unload churn)
- Throughput stats live: `throughput.py` tracker records per-window latency; `/health` + `/system/status` expose windows + avg latency + concurrency; System tab shows a "Classifier throughput" card

**Expected:** 3-5x wall-clock reduction on GPU machines; scales with hardware. Without `OLLAMA_NUM_PARALLEL`, requests queue at the server (no speedup, no harm).

### B12. Retrieval quality: hybrid search + chunk cleaning (P2 — discovered, confirmed live) — DONE (build); re-test on rulebook
**Problem:** full-document chunks are raw extracted text (repeated headers, page numbers, encoding garbage) and vector-only search with `nomic-embed-text` ranks them poorly — a DVCA question scored all candidates ~0.7 and surfaced unrelated sections, while clean entity-summary chunks retrieved far better. Verified with a retrieval probe on the 237-page rulebook (only 5 chunks mention DVSE/DVCA; they ranked below unrelated chunks).

**Done (build):**
- **Chunk text cleaning** (`cleaning.py`): strips control chars/soft hyphens/replacement chars, page-number lines ("Page 3 of 12", "- 42 -"), repeated header/footer lines (verbatim lines above a doc-size threshold), normalizes whitespace/blank runs. Applied before full-document chunking so boundaries, embeddings, and FTS tokens are clean.
- **Heading-aware chunking**: `chunk_document` now splits at heading lines (`§434`, `4.2.1`, `Article 12`, ALL-CAPS titles), accumulates paragraphs within a section up to `ANCHOR_CHUNK_MAX_CHARS` (default 1600, within nomic-embed's context), fixed-size split only for oversized paragraphs.
- **Hybrid retrieval**: new `chunks_fts` FTS5 table (external content + sync triggers + backfill; migration `b12f7c0`). `AnswerEngine` merges cosine vector scores with bm25 keyword scores at `ANCHOR_RETRIEVAL_KEYWORD_WEIGHT` (default 1.0; 0 = pure vector, 1 = keyword full weight in RRF). Acronyms like DVCA/DVSE hit directly via FTS. Newest-chunks fallback kept for pre-migration DBs.
- New tests: cleaning, heading chunking, FTS table existence, end-to-end acronym retrieval (`tests/test_retrieval.py`).

**Live probe (Aug 2026, after reclassify of the rulebook):** "What are the DVCA movement rules?" → §372 (4.25 DVCA) at **0.931**; "Merger and related movements" → §434 (4.39 MRGR) at **0.892**; both top-ranked with correct sections ≥ 0.8 (previously ~0.7 with unrelated sections surfacing above the real ones, plus a hallucinated PLAC→merger link from llama3.2:3b).

**Remaining (optional, evaluated after hybrid):** LLM rerank of top-k candidates; re-test answer quality with a better generation model than llama3.2:3b (retrieval is now the strong part; generation still limits final answers).

### B12.1. Fusion + diversity + recency + context expansion (P1 — Cerebras learnings) — DONE
**Problem:** the hybrid merge used score normalization (min-max), which is fragile across retrievers; one large document could monopolize top-k; old and new answers ranked equally; chunk boundaries stripped surrounding context.

**Done (informed by [Cerebras' KB post](./architecture.md#4b-retrieval-design-informed-by-cerebras-knowledge-base)):**
- **RRF fusion** replaces min-max: `score = Σ w / (60 + rank)` across vector + FTS5 lists; consensus across retrievers beats a single strong vote; no normalization
- **Age decay**: `0.5^(age/halflife)` multiplier (`ANCHOR_RETRIEVAL_AGE_HALFLIFE_DAYS`, default 365) — newer answers win ties
- **Per-source diversity cap**: max 3 chunks per source (`ANCHOR_RETRIEVAL_MAX_PER_SOURCE`) — one document can't monopolize results
- **Context expansion**: after ranking, pull up to `ANCHOR_RETRIEVAL_CONTEXT_WINDOW` (default 1) neighboring chunks per side of a winning chunk, so headings/preconditions/caveats aren't lost
- Tests: RRF consensus vs single vote, keyword weight scaling, age decay math, per-source cap

**DoD:** two sources answering the same question — neither monopolizes top-8; a newer answer beats an older one at equal relevance; answers include neighboring section context.

**B12.1 fixes (post-build, live findings):**
- Diversity cap is now **per-file (item)**, not per-source: a multi-file corpus gets diversity (Cerebras: "cap how many results each file can contribute"), but a single big document is no longer starved (cap relaxes when the candidate set has <3 distinct files)
- **Near-duplicate chunk dedupe**: same content appearing twice (TOC entry + body section, e.g. §720/§434 MRGR) keeps only the best-scoring copy
- **Synthesis prompt strengthened**: the model must synthesize process/workflow steps explicitly from evidence instead of hedging ("not explicitly described") — live probe: "how does MRGR work" now returns a step-by-step explanation citing sections, where before it refused
- New tests: per-item cap with multiple files; cap relaxed for single-file corpus

### B13. Dev process hardening — DONE (commit: add B13 dev process hardening)
**Problem (from session retrospective):** environment chaos cost more time than code bugs — multiple servers sharing one SQLite DB (stale code, lock fights, an elevated process we couldn't kill), `.env` silently overwritten with example defaults, duplicate keys silently overriding (pydantic takes the last), 15-min operations looking frozen, hand-rolled SQLite migrations already biting once.

**Done:**
- **Single run entry point**: `start.ps1`/`start.sh` that kills orphans on the port, creates the venv/.env on first run, applies Alembic migrations, and boots the backend
- **Startup config validation**: `.env` warnings for duplicate keys + unknown `ANCHOR_*` keys; boot banner logs effective settings (base URL, models, DB path); Ollama model availability checked at startup
- **Alembic migrations** (replace hand-rolled `db.py:migrate` ALTERs) — legacy DBs auto-stamped
- **CI via GitHub Actions**: pytest + frontend build on every push
- **B2 pulled forward**: sync/reclassify are now background jobs (202 + `GET /sources/jobs`), Sources tab polls progress with busy states and inline progress bars

### B10. Model configuration via UI — superseded by B4. (see B4)

### B15. Scoped search via projects (P2 — Cerebras learning) — DONE
**Problem:** as the corpus grows, "search everything everywhere" stops being useful — compiler engineers don't want infrastructure runbooks in their results. Cerebras found projects are how search stays relevant by default.

**Done:**
- **Schema**: `projects` + `project_sources` (many-to-many; a source belongs to multiple projects; `is_default` marks the user's default scope) — migration `b15a0d1`
- **API**: `GET/POST /projects`, `GET/PATCH/DELETE /projects/{id}`, `GET /projects/default`; sources validated before any write (a 422 leaves the DB untouched)
- **Retrieval scoping**: `POST /qa` accepts `project_id`; `AnswerEngine` threads a project's source ids through all tools (vector, FTS, who_knows, graph, fallback) so only that project's sources are retrieved
- **UI**: project picker in the header (scopes Ask; ★ marks the default), plus a Projects manager in the Sources tab (create / assign sources / make default / delete)
- **Tests** (`tests/test_projects.py`): CRUD, default singleton, unknown-source 422, and the **DoD** — two projects with an overlapping source return project-scoped citations

**DoD:** connect 3+ sources, create two projects with overlapping sources, and confirm the same question returns project-scoped results. — met (live: a "Payments only" project excludes an unrelated parking source from citations).

### B16. who_knows — expertise queries (P2 — Cerebras learning)
**Problem:** a top question in every org is "who is the expert in Y?" Cerebras surfaces people with demonstrated expertise from the index.

**Scope:**
- Query over entity graph + owners: entities authored by/assigned to a person, weighted by entity confidence + recency + count
- `who_knows(topic)` tool (MCP + UI); people surfaced with their evidence (which entities prove expertise, cited)

**DoD:** ask "who knows about migrations" and get a ranked person list with cited evidence.

### B17. Planner → Executor → Synthesis query architecture (P2 — Cerebras learning) — DONE
**Problem:** today Q&A is single-pass RAG (one retriever set, one synthesis call). Cerebras runs a light planner pass that picks which retrieval tools matter for the query, fans them out in parallel, normalizes evidence, then synthesizes.

**Done:**
- **Planner** (`AnswerEngine._plan_tools`): deterministic, model-free tool selection — `hybrid` (vector + FTS5) always; query signals (`who/whom/owns/owner/responsible/expert/knows`) add the `who_knows` specialist tool. CI-safe; an LLM planner can be layered on later without changing the executor contract.
- **Executor** (`_execute_tools`): runs the planned tools, batching the shared embedding into one call, and normalizes each tool's hits into a shared evidence shape (chunk/entity/item/source_id/score).
- **`who_knows` tool**: surfaces entities whose owner/author/summary matches the query, ranked by confidence × recency × term overlap — person + ownership evidence a plain keyword search doesn't weight.
- **Synthesis** (`_fuse_evidence`): RRF-fuses the tools' ranked lists (hybrid internally fused; who_knows at weight 0.8; graph-connected entities at 0.5), dedupes, tops off at top_k. `_rrf_fuse_multi` generalizes the two-list RRF to N lists.
- Tests (`tests/test_planner.py`): planner always hybrid; who_knows for ownership questions; who_knows surfaces owner entities; executor returns evidence bundle; multi-tool fusion; **DoD test** — a decision+person question returns both cited.

**DoD:** a question that needs two source types (e.g. decision + person) is answered with evidence from both, cited. — met (live: "Who owns the billing migration?" → planner runs `['hybrid', 'who_knows']`; answer cites Sarah's ownership + the billing decision/action).

### B18. Distillation of raw content before embedding (P2 — Cerebras learning) — DONE
**Problem:** Cerebras found embedding raw text underperforms a normalized form: "accuracy increased significantly when the thread was normalized into a consistent format." They extract question/summary/resolution/system refs from threads; short filler messages beat detailed ones in cosine similarity.

**Done:**
- **Distillation pass** (`Classifier.distill` + `pipeline._distill_and_store`): chat-like windows (doc_type `meeting|decision_log|general`) are normalized into searchable Q&A units — `{question, answer, terms, systems}` — stored as first-class chunks with `kind='distilled'` and a normalized `Q: … A: …` content. Hash-skipped per window (re-sync/reclassify of unchanged content makes ~0 distillation calls). LLM path with a rule-based fallback (CI-safe).
- **IDF-gated embedding** (`_signal` + `embed_min_signal`): chunks whose vocabulary is low-signal (short filler, rare-token sparse) are skipped from vector embedding — they stay `embedding = NULL` so FTS5 keyword search still finds them.
- **Chunk discriminator**: new `chunks.kind` (`document`|`entity`|`distilled`) — migration `b18d0c1` backfills existing entity chunks.
- **DoD met live**: a Slack-export chat produced 3 distilled units; "How long does the idempotency key last?" answered from the distilled unit ("the idempotency key expires after 24 hours") even though the raw thread phrased it as "what's the timeout on the idempotency key?"

**DoD:** a chat-log-style source produces findable distilled Q&A units; filler messages don't pollute vector results. — met.

### B19. Chat: newest answer on top (P2 — UX) — DONE
**Problem:** in long follow-up conversations the latest answer renders at the bottom of the page, off-screen; the user has to scroll down to see the new response (or misses that it arrived).

**Done:** Ask tab renders turns newest-first (latest turn at the top, below the input); input stays pinned near the top, older context scrolls down, "Clear context" stays visible.

**DoD:** after 5+ follow-ups, the newest answer is visible without scrolling. — met.

### B20. Launchable package for others (P1 — "usable by others" blocker) — DONE
**Problem:** today AnchorCore runs from a repo checkout (`start.ps1`); a non-developer can't install and launch it. `packaging.md` is a plan only, stale (references Next.js; we're Vite), and macOS-only.

**Done:**
- PyInstaller onedir windowed backend app bundling the built `frontend/dist` — one process serves UI + API on 127.0.0.1:8000 (`dist/AnchorCore-windows.zip` → `AnchorCore.exe`; macOS `dist/AnchorCore-macos.zip` → `AnchorCore.app` via `build.sh`, B24)
- Launcher (`backend/run_app.py`): sets `ANCHOR_DATA_DIR=~/.anchorcore` when frozen, auto-starts the local Ollama server, opens the browser, boots uvicorn; windowed-mode guard redirects stdout/stderr (no terminal)
- `packaging.md` updated to reality (Vite, current stack, both platforms); notarization deferred as the stretch milestone
- CI artifact: `.github/workflows/release.yml` builds the exe/app on every `v*` tag (B25) so testers get a download

**DoD:** a non-developer downloads one archive, runs it, and lands in the Ask tab with models working — no terminal, no repo. — met (v1.0.0+ Windows + macOS releases on GitHub).

### B21. Sample dataset for variety and complexity (P1 — first, per agreed prio) — DONE
**Problem:** `sample/` has one 18-line sprint note. Can't demo or validate variety (classification kinds, relationships, duplicates, review, follow-ups, people) on real-shaped data.

**Done:**
- Mini-company corpus: `decisions/` (PRD cuts, architecture tradeoffs w/ reasoning), `meetings/` (retro + planning notes with action items + owners), `specs/` (feature doc with cross-references), `people.md` (who owns what — feeds who_knows)
- Deliberately includes: one duplicate pair (review/merge), one low-confidence item (review), one disputed-worthy claim, and follow-up-friendly topics
- `docs/sample-dataset.md` — how to point a folder source at `sample/` and what each file exercises + demo scripts

**DoD:** connecting `sample/` exercises every Review tab state, produces multiple entity kinds with owners, and supports a 3+ turn follow-up demo. — met (live demo flows).

### B22. Live integration validation: Jira / Linear (P2)
**Problem:** the Jira connector has never hit a real instance; Linear doesn't exist. "Connects to your tools" is claimed but unproven.

**Scope:**
- Jira: validate against a real sandbox instance (Atlassian free tier + API token) — incremental cursor, comments→doc text, author/assignee mapping, error surfaces; fix whatever breaks
- Linear: new connector (Linear API key auth, issues/cycles → IngestionDoc), incremental by updatedAt, same source config pattern (base_url/api_key/project→team)
- Recorded JSON fixtures + mocked-fetch tests so CI validates connector logic without credentials (per B21 sample data)
- Troubleshooting pass in README: how to create a Jira API token / Linear API key, expected permission scope

**DoD:** a Jira source and a Linear source sync real data end-to-end on a sandbox; connector logic covered by fixture tests in CI.

### B23. External/cloud models for classification (P1 — raised from P2: unblocks complex classification without a big local model) — DONE
**Problem:** the classifier is hardwired to Ollama (`classifier.py` posts to `ollama_base_url`); B4 lets users set the *answer* model to any OpenAI-compatible provider, but classification can't. Better models = more accurate complex classifications (documents with tricky kinds, higher confidence).

**Done:**
- Classifier decoupled from Ollama: `classifier_base_url` (empty → Ollama, unchanged default) + `classifier_api_key` (SecretStore, `***set***` masked) — Bearer auth when a key is present
- Settings tab: "Classifier provider" section (Local Ollama / Cloud OpenAI-compatible), model + base URL + key, **Test classifier** button, cost guardrail note (classification calls per document window; a 274-page doc ≈ 160 calls)
- `/system/status` reports classifier `provider: local|cloud`, base URL, model (merged with throughput stats); Ollama model-missing warnings only apply to the classifier when it's local
- Tests: cloud classifier posts to own URL with Bearer; local default uses Ollama without auth; classifier key masked in API

**DoD:** a user points classification at a cloud model from the Settings tab; a complex document classifies with higher confidence than the local 3B, no restart. — met (runtime-mutable via SettingsService; reclassify a source to apply).

**B23 follow-up (done):** embeddings got the same treatment — `embed_base_url` + `embed_api_key` (keychain), Settings "Embeddings" section with provider dropdown + always-visible API key + model, `Test embeddings` button, System tab Embeddings card. Settings tab restructured so every model section (classification / embeddings / answer) is self-contained: provider select (Local Ollama / Cloud), API key, base URL, model — no more conditional reveals. Changing the embed model requires re-embedding (reclassify or backfill).

### B26. Reviewable, document-aware classification (P1 — tester feedback) — DONE
**Problem (live feedback on the 274-page rulebook):** (1) the classifier was type-agnostic — a regulatory standard produced spurious "decision"/"action" labels for procedural steps ("Notify Account Owner", "Cancel"); (2) the Review tab showed only a one-line summary with no source context, so it was impossible to judge correctness; (3) 8k-char windows meant ~160 cloud LLM calls per reclassify.

**Done:**
- **Document-type pre-pass**: one cheap call per document detects `standards|runbook|meeting|decision_log|prd|general`, cached on the item (`doc_type`); the extraction prompt adapts per type — standards/runbook prompts explicitly suppress decision/action and extract only `note` (rules, definitions). Live check: the 237-page rulebook now classifies as `standards` → **99 note entities, zero decision/action** (was ~40% action/decision).
- **Reviewable entities**: `entities.window_text` + `window_index` store the exact classifier input window; Review tab shows "Show what the classifier saw" — an expandable source-excerpt panel per entity (migration `b26a0c2`).
- **Cheaper cloud classification**: default window 8k → 16k chars (~halves API calls); per-window content hashes (`item.window_hashes`) — incremental syncs and reclassify skip unchanged windows (force reclassify = full rebuild); no more hard 12k truncation.

**DoD:** reclassifying the rulebook labels it standards and produces only note entities; a reviewer sees the source excerpt behind every low-confidence entity; a second reclassify of unchanged content makes ~0 classifier calls.

### B27. Review page: real context, not just the window (P2 — tester feedback)
**Problem:** even with `window_text`, the Review page still shows a raw classifier-window excerpt — no surrounding document context, no neighbouring sections, no file reference. Reviewing an entity still feels like judging a fragment in a vacuum ("small chunks, no value").

**Scope:**
- Review card shows: entity summary + kind + confidence + **source file name/link** + the section it came from (`source_ref`)
- **Context expansion in review**: render the entity's window *plus* the neighbouring sections from the same item (reuse `_expand_context`-style logic / `chunk_document` sections), so the reviewer sees the whole surrounding passage
- **Highlight the entity's summary text** within the source excerpt when it appears verbatim; if not verbatim, show the window with the summary quoted above it
- Quick-actions stay on the card (Looks right / Dispute) but with a "full document" affordance — e.g. expandable full source text (collapsed by default, 16k windows are heavy)
- Same treatment for the Entities tab card

**DoD:** reviewing a low-confidence entity shows the file, section, surrounding paragraphs, and the exact text the classifier summarized — a reviewer can judge correctness without opening the source file.

### B28. Google Drive connector (P2 — big real use case)
**Problem:** most teams keep shared docs in Google Drive (folders, shared drives). AnchorCore's folder connector only watches local disks — Drive content requires manual download. Candidate simple path: "access the folder → download contents into a local sync folder".

**Scope:**
- **Simple path (recommended first): Drive → local sync folder.** Connector authenticates (OAuth or app password), maps a Drive folder/shared drive to a local mirror dir (e.g. `data/drive/<name>/`), downloads new/changed files on a poll, then the existing folder connector ingests the mirror. Incremental via Drive modifiedTime; deletions → stale
- Native alternative (later, if needed): Drive API list+download directly into the pipeline without a local mirror
- Auth: OAuth flow (needs Google Cloud project + client id) or service-account-less per-user token; store token in SecretStore
- Supported types: Drive-native docs need export to .txt/.pdf (Google Docs → txt/pdf, Sheets → csv, Slides → pdf)
- Troubleshooting/README: how to create a Google Cloud project + OAuth consent for personal use

**DoD:** a user authorizes a Drive folder, AnchorCore syncs its docs (including Google-native formats), and Q&A answers cite Drive sources with working file links.

### B29. v2/v3 business & platform scoping (P3 — see [v2v3-scope.md](./v2v3-scope.md))
**Problem:** v1 is local-first and free; there's no website, hosted path, pricing, or changelog discipline — nothing for people to find/try/pay for.

**Scope (per [v2v3-scope.md](./v2v3-scope.md)):**
- Website: landing (60s demo), download, docs site, pricing, changelog page
- Hosting: same code, env-driven — Postgres + object storage + server-side connectors; single-region VPS first
- File sharing: read-only share links (project tokens) → collaborators → permissions (the Teams tier trigger)
- Release mgmt: mandatory CHANGELOG.md + SemVer; update-checker later
- Free tier + pricing: local forever free (privacy moat); hosted paid; small hosted free tier as the no-install demo
- Enterprise (v3 stretch): SSO, audit logs, on-prem

**DoD:** a stranger lands on the website, downloads the app (or starts the hosted free tier), and asks a cited question in under 3 minutes; a changelog accompanies every release.

**Decisions 2026-08-10:** B29 is operationalized by B33–B37 — website (B33), telemetry (B34), hosted tier + pricing (B35), BSL licensing (B36), on-prem enterprise (B37).

### B14. Agent connectivity via MCP (P2 — see [mcp.md](./mcp.md))
**Problem:** users want their own harnesses (Claude Code, Codex, opencode) to use AnchorCore's memory, but today only the browser UI can reach it.

**Scope (per [mcp.md](./mcp.md)):**
- B14.1 Local stdio MCP server, read-only: `ask`, `search`, `get_entity`, `get_source`, `list_sources`, `memory_status` — thin adapters over existing services; results respect status/dispute filtering — **DONE (v1.0.9)**
- B14.2 Streamable HTTP transport mounted on the FastAPI app (`/mcp`), bearer-token auth (opt-in, disabled on localhost), calls audited in `system_events` — the "centralized dataset for agents" story
- B14.3 Write-back `ingest` tool (items stored `unverified`, author `mcp:<token>`, routed to the review UI)
- B14.4 Registry publishing for one-command harness installs

**DoD:** a tester points Claude Code at their AnchorCore memory (local or centralized), asks a question, and gets a cited answer; every MCP call appears in the audit trail.

### B30. Data labeling: PII/sensitive gating of models, sharing, and answers (P1 for PII — v1 risk, P2 rest)
**Problem:** cloud classification exists (B23) — but nothing stops PII or sensitive content from being sent to a cloud model, or surfaced in shared/agent-facing answers. For team use (v2) this is a hard blocker; even locally it's a trust story ("what leaves my machine?").

**Concept — labels on sources, gates on everything else:**

1. **Labels** (`sources.label`, default `internal`):
   - `public` — safe to share/answer via links, MCP, agents
   - `internal` — default; local + trusted-cloud models OK, not shareable
   - `sensitive` — local models only (no cloud classifier/embedder/answer), never shareable
   - `pii` — local models only; contents may contain personal data; redaction emphasis
   - (v2.5: per-item/per-entity labels, auto-detection via NER as a stretch)

2. **Provider trust tiers** (Settings tab): each provider (classifier/embedder/answer) declares a trust level — `local` (Ollama = highest) vs `cloud` (user-confirmed "I accept sending data to this provider"). A source label requires a minimum trust tier for each model step.

3. **Gates:**
   - **Model routing**: pipeline refuses to classify/embed `sensitive`/`pii` content with a provider below the required trust tier → falls back to local Ollama or rule-based, records a `system_event` warning ("blocked cloud classify on pii source X")
   - **Sharing/answers**: Q&A, share links (B15/B29), and MCP tools exclude non-`public` sources unless the session is authenticated + authorized (single-user local = everything visible; hosted/team = label-scoped)
   - **Audit**: every gate decision (allow/block) lands in `system_events` with source + label + provider

**DoD:** label a folder `pii` → cloud classifier/embedder/answer never touch it (local-only, verified in logs + a `system_event`); a shared link (v2) answers only from `public` sources; the UI shows each source's label and what it allows.

**B30 open question (needs product input before implementation — agreed 2026-08-07):**
What counts as PII, and how do we map labels onto content? Proposed default to ratify:
- **Hard PII (never leaves local)** — anything mappable to an identifiable person: names + contact (email, phone, address), government IDs (SSN/passport/driver's license), financial identifiers (account/card numbers), HR/medical data, credentials/secrets. Source label `pii` → local models only, excluded from sharing/answers/MCP.
- **Sensitive (soft PII)** — data that isn't person-identifying but is commercially/strategically sensitive: customer lists, pricing, unreleased plans, legal drafts, security posture. Label `sensitive` → local-only, not shareable, but may be answerable to an authenticated local session.
- **Internal** — default; local + trusted-cloud OK; not shareable.
- **Public** — safe to share/answer via links, MCP, agents.
- **Mapping question**: do we label at the **source level** only (folder = `pii`, so everything inside is gated — simple, safe, coarse), or allow **per-file/per-item overrides** (a docs folder containing one HR file)? Recommendation: source-level first (v1 semantics are "trust the source label"), per-item auto-detection (NER for emails/IDs/names) as a v2.5 stretch — never auto-*downgrade* to a less-restrictive label.

### B24. macOS build + ad-hoc signing (P2 — free path, no $99) — DONE
**Problem:** Windows has a distributable exe (B20); macOS has none. The paid Apple Developer account ($99/yr) is only needed for *notarization* (silent Gatekeeper approval); for personal use and testers who accept one-time approval, a free path exists.

**Scope (build runs on the Mac — PyInstaller doesn't cross-compile):**
- `build.sh` mirroring `build.ps1` (npm build → pyinstaller → output)
- Spec adjustments for macOS: sqlite_vec glob picks up `.dylib` (not just `.dll`); Ollama path candidates already handled in `run_app.py` (`/usr/local/bin/ollama`, `/opt/homebrew/bin/ollama`)
- **Ad-hoc sign**: `codesign --force --deep --sign - dist/AnchorCore` (free, no account)
- Distribution as zip (or dmg later); README note: recipients approve once via right-click Open / System Settings → Privacy & Security → Open Anyway / `xattr -d com.apple.quarantine`
- Free tier = "approve once per machine"; defer notarization ($99) until real distribution is needed

**Done:**
- `build.sh` — mirrors `build.ps1` (frontend build → venv/pyinstaller → `dist/AnchorCore` → ad-hoc sign)
- `packaging.spec` — sqlite_vec glob picks up `.dll` *or* `.dylib` (Windows + macOS from one spec); verified `vec0.dylib` bundles correctly
- `.github/workflows/release.yml` — macOS job zips `dist/AnchorCore.app` (windowed onedir bundle) so the tag build doesn't fail
- Smoke-tested on a Mac (arm64): boots, migrations run, syncs the `sample/` corpus (10 files → 47 entities), QA returns cited answers, Settings switch to local Ollama produces generated answers

**DoD:** running `./build.sh` on a Mac produces a launchable, ad-hoc signed `dist/AnchorCore`; launching on a fresh Mac works after the one-time approval.

### B25. Release step: rebuild executables on every release (P1 — process) — DONE
**Problem:** the packaged exe embeds the frontend at build time, so stale builds silently miss UI changes (the B23 "missing API key field" incident). Rebuilds were manual and easy to forget.

**Done:**
- `.github/workflows/release.yml` — on every `v*` tag push: builds Windows exe (Windows runner) + macOS app (macOS runner, ad-hoc signed, zipped), attaches both to the GitHub Release
- `docs/releasing.md` — release checklist (tests → version bump → tag → verify artifacts → hand off) and the "why rebuild is mandatory" note

**DoD:** tagging `v0.x.0` produces downloadable Windows + macOS artifacts automatically, verified by a smoke test on a clean machine.

### B31. Backend port to Rust (P3 — deferred; see [rust-port.md](./rust-port.md))
**Problem:** Python fully packaged is rough for non-technical testers — PyInstaller/venv
fragility (we've shipped two packaging bugs: `.dylib` glob, toolcache-Python lacking
loadable sqlite extensions), no cross-compile, cold start. The question keeps coming up:
*"would Rust make it easier to run and faster?"* — this is the standing evaluation + plan.

**Short answer (in [rust-port.md](./rust-port.md)):**
- **Easier to run: yes.** One static binary, no runtime, cross-compiles. This is Rust's
  real win and it removes exactly the failures that reach testers.
- **Faster: mostly no.** LLM inference (Ollama/cloud) dominates latency and is
  language-independent. Rust only speeds retrieval/chunking — which is achievable in
  Python via the bundled sqlite-vec `vec0` index (~10–100x, ~1 day).
- **Ease of support:** Rust trades iteration speed for runtime reliability. Fair for a
  solo maintainer *only if* the port is incremental (same API + SQLite schema contract,
  both sides run the shared test suite) — never a big-bang rewrite.

**Do cheaper wins first (any language):** bundled llama.cpp inference (the *actual*
"works for non-techies" fix, already on `packaging.md` open work), `vec0` retrieval index,
first-run wizard (B8). If those satisfy testers and releases stop breaking, don't port.

**When the port IS worth it:** testers can't install Ollama (fix with bundled inference,
not a rewrite); releases keep breaking on packaging (2 incidents already — Rust's biggest
win); retrieval latency on real corpora matters after vec0.

**DoD:** Phase 3 of the plan — a Rust binary serving the same REST API and SQLite file
as the Python backend, with the existing test suite green against it, demoable and
rollback-safe — OR a documented decision to stay on Python.

### B32. Graph-based retrieval: relationship-aware rerank + expansion (P2 — internal-tool parity) — DONE
**Problem:** AnchorCore builds the entity graph (`entities` + `relationships`: supersedes/depends_on/owns/blocks) but never consults it during Q&A — retrieval is vector + FTS5 RRF only. An internally comparable tool does vector *and* graph-based retrieval, so questions that RRF alone answers poorly ("what supersedes this?", "what depends on this decision?") fall through because the words don't co-occur. The graph already exists; it's just not wired into the answer path.

**Done:**
- `AnswerEngine._graph_expand` — after RRF fusion, walks `relationships` 1-2 hops from the winning entities and pulls connected entities' summaries/chunks into the answer context as `[related]` sections + citations
- **Traversal weighting**: supersedes (1.0) > depends_on (0.9) > owns (0.8) > blocks (0.7) > related (0.4); hop decay `0.5^hop`; fan-out capped by `retrieval_graph_max` (default 3) — a hub entity can't flood context
- Connected entities render as citations with their own summary + source ref; stale entities excluded (same filtering as vector/keyword retrieval)
- Config: `retrieval_graph_hops` (2), `retrieval_graph_max` (3); reflected in the boot banner
- Tests (`tests/test_graph_retrieval.py`): connected entities surface; strong kinds rank above related; stale excluded + fan-out capped; kind weights sane; Q&A returns graph citations

**DoD:** asking "what superseded the security-transfers decision?" (or a natural phrasing the model paraphrases) returns the newer decision's summary as a cited, connected result without the words co-occurring in both documents. — met (live: "Who owns the billing migration?" surfaces the connected security-transfers decision at score 0.9).

### B33. Website: landing page + docs + changelog (P1 — GTM, decided 2026-08-10)
**Problem:** v1 has no findable presence — a stranger can't find AnchorCore, understand the 60-second wow, or download the artifacts. B29 scoped it; this locks the decisions.

**Decisions (2026-08-10):**
- Domain: **`anchorcore.dev` registered** (~$12/yr, Cloudflare Registrar). `anchorcore.ai`/`.com`/`.app` are taken but parked (squatters) — no premium paid.
- Stack: static Astro/Vite site on Cloudflare Pages (free) — no CMS, no backend. Docs site later via VitePress or rendered repo docs.

**Scope:**
- Landing (one page): hero *"Connect your knowledge to any AI model"* + a 60s screen recording of the sample-corpus demo (connect → ask → cited answer), 3-step how-it-works (connect a folder/Jira → it learns: classify, graph, citations → ask with cited answers), download CTAs (Windows exe + macOS app from GitHub Releases), pricing skeleton (local free / hosted later), docs + changelog links, and the privacy line: *"local-first — your data never leaves your machine."*
- Changelog discipline (B29 §5): mandatory `CHANGELOG.md` per release, rendered on the site.
- Pricing page: "Free during validation" until hosted exists; becomes the B35 tier skeleton later.

**DoD:** a stranger lands on anchorcore.dev, downloads the app, and asks a cited question in under 3 minutes; every release updates the changelog page.

### B34. Opt-in usage telemetry (P2 — validation signals)
**Problem:** we can't retroactively learn who'd pay (which features get used, where the value cliff is), and "local forever private" vs "collect usage" look contradictory.

**Decisions (2026-08-10):** telemetry sends **counts, never content**. Off by default; one explicit first-launch prompt (default unchecked). The contract: "anonymous usage stats — never your content, sources, queries, or answers."

**Scope:**
- Counts-only events: source types connected, questions/week, errors by type, feature engagement (MCP enabled, share links created, quota caps hit), app version. Zero entity text, zero file paths, zero queries/answers — if a field could contain content, don't send it.
- Pseudonymous random UUID per install, reset-able ("reset my telemetry ID" button); aggregates only, no per-user tracking.
- Transparency as a feature: Settings screen listing the last events and every field they contain. Local-only stats view available even when sending is off (reuses `system_events`).
- Public-signal complement: GitHub release downloads, repo stars, docs analytics — funnel shape without touching user data.
- Enterprise: instance-health reporting becomes a paid support feature (B37), not consumer telemetry.

**DoD:** a user sees exactly what would be sent, can decline by default, and still gets value from local stats; we get aggregate feature-usage counts (e.g. "8 users connected a Jira source this week").

### B35. Hosted tier: low-risk features, cost bounds, security (P3 — when hosted is real)
**Problem:** hosting risks unbounded costs; only some features have predictable cost curves. We narrowed v2v3-scope §6 into a concrete, quota-bounded offering.

**Decisions (2026-08-10):**
- **Sub-only pricing — one dial, no double-billing.** The sub includes everything local + hosted; there is no separate fixed license for the app (two revenue models confuse buyers). Prices: Individual **€19/mo**, Teams **€25/user/mo** (narrowed from v2v3-scope §6's €15–50 / €20–50).
- Local app stays **free forever** (privacy moat + word of mouth); free tier exists to cross the download → value gap, not to subsidize heavy hosted use.

**Low-risk hosted features** (quota-bounded by design — nothing here is real-time multi-user compute): hosted sync (Jira/Drive poll), public MCP endpoint (B14.2 — agents can't hit a laptop behind NAT), webhook ingest, read-only share links (B15 projects + token links), collaborative review (storage-only, no model calls).

**Cost bounds (the circuit breakers):**
- Storage quota per workspace (e.g. 500 MB); sync frequency cap; ingest quota shared between sync + webhooks.
- **Object storage: Cloudflare R2 (locked 2026-08-10)** — S3-compatible (existing swap point), free tier: 10 GB storage, 1M Class A (write) + 10M Class B (read) operations/mo, **$0 egress** (read-only share links serve from R2 at zero bandwidth cost). At the 500 MB/workspace quota that's ~20 free workspaces; overage is cheap (~$0.015/GB/mo). R2 is storage only — compute + Postgres live on the VPS (D9).
- **Metered monthly token allowance** (~2–3M tokens) with **cheap-tier bundled models only** (Flash/Haiku class — never Sonnet/o-series bundled; expensive models = BYO key, existing B4 pattern).
- Embedding content-hash cache (reuse the window-hash dedupe from B26) so re-syncs don't re-embed.
- Per-workspace + per-MCP-token rate caps — a misbehaving agent script can't rack up €50 in a night.
- Rough math: ~1,000 questions/mo on a cheap model ≈ $2–3/workspace/mo → €19 is 4–7× margin.

**Security/privacy:**
- **B30 label gating is the privacy answer**: hosted workspace ingests `public`/`internal` sources only; `sensitive`/`pii` stay local-only (hard pipeline rule, not policy).
- Per-workspace row isolation (generalize the `projects`/`source_ids` scoping), no cross-tenant leaks.
- Scoped MCP API tokens: read-only vs write, per-token revocation, every call in `system_events` audit trail.
- Encryption in transit + at rest; secrets in a real secret manager (SecretStore interface swaps cleanly); GDPR export + delete workspace endpoints **before** the first EU customer.

**DoD:** a hosted workspace's worst-case monthly cost is bounded by its allowance and quotas; a heavy user cannot blow up infra; a `pii` source never reaches the hosted tier.

### B36. Commercial licensing: BSL 1.1 + signed license keys (P1 — legal prerequisite)
**Problem:** the repo has no LICENSE — legally "all rights reserved", so nobody (including testers) can use it, and there is no mechanism to sell on-prem deployments. This is upstream of B37 and the pricing story.

**Decisions (2026-08-10):**
- **BSL 1.1** (the MariaDB/Sentry pattern): `LICENSE` file at repo root — Additional Use Grant = non-commercial + orgs <10 employees / <€1M revenue use free; any other commercial use requires a license from the Licensor; Change Date = 4 years after first publish; Change License = Apache 2.0.
- **Ed25519-signed license keys, verified locally — works offline** (air-gapped enterprises are the best customers; they can't phone-home). The signing private key never ships; the app embeds only the public key, so keys can't be forged even by extracting the binary.
- Enforcement is friction for the honest 90%, not a fortress; big enterprises have legal departments that won't run an unlicensed deployment.

**Scope:**
- `LICENSE` (root, BSL 1.1 template filled in).
- `tools/make_license.py` — private keygen: `--edition enterprise --licensee "Acme" --expires 2027-01-01` → base64 key (keep out of the repo / private repo / CI secret).
- `backend/app/licensing.py` — verify key against embedded public key, parse claims (edition/licensee/expires/features), expose `is_enterprise()`.
- `GET /system/license` — edition, expires, unlocked features (mirrors `APP_VERSION` in `routers/system.py`).
- Settings tab "License" section — paste key, runtime-mutable (B4 pattern), no restart.
- Feature gates `if licensing.is_enterprise():` for future enterprise routes (SSO, audit export, RBAC — B37).
- Tests: valid key unlocks; tampered/expired key rejected; community build works keyless.

**DoD:** a client's signed key unlocks the enterprise edition offline; a tampered/expired key is rejected; the community build runs without a key; the LICENSE file makes commercial use require a license.

### B37. Enterprise on-prem platform (P3 — v3)
**Problem:** big clients won't run a PyInstaller exe — they want turnkey server deployment. Strategy: **sell the platform, don't host it** (zero infra risk for us; a sales-led annual contract instead of a hosting bill).

**Scope:**
- Docker image + compose (Postgres + object storage swap per v2v3-scope §3), one-command deploy.
- Managed updates via license-key phone-home ("update available, one click"); health/metrics endpoint (`/health` + counters) their ops can wire into their stack.
- **Multi-user login + RBAC** (biggest gap — the app is single-user today; MCP bearer tokens from B14.2 are the scaffolding), SSO (SAML/OIDC), audit log export.
- **Enterprise telemetry = SLA feature**: instance-health reporting as part of the support contract (consumer telemetry stays minimal, B34).
- Pricing: **flat annual per deployment** (not per-seat at first — one sales conversation beats a pricing page): €2.5k–5k/yr core (deploy + updates + support), €15k+/yr with SSO/audit/RBAC unlocked, bespoke €20k–100k/yr.

**DoD:** a client deploys with one docker command; SSO + audit export work; the contract is a renewal conversation, not a hosting bill.

### B38. Design & brand work (P1 — GTM, feeds B33)
**Problem:** everything discussed so far is strategy and product; none of it is *design*. A landing page that looks template-y and a functional-but-plain UI undercut the positioning ("AI memory layer") and the 60-second wow. This item owns the visual layer that B33's landing, the demo, and the download page all depend on.

**Scope:**
- **Brand identity**: name/wordmark, logo (anchor motif fits "AnchorCore" + memory positioning), color palette, typography, tagline lock-up. Positioning reference: *"Connect your knowledge to any AI model."* Deliverables in `design/` in the repo (or Figma — decide once).
- **Landing page visual design** (input to B33): hero, 3-step how-it-works visual (connect → it learns → ask with citations), pricing card design, privacy line treatment.
- **60-second demo**: screen-recording script of the sample-corpus flow (B21) — connect `sample/` → ask "what was decided about X, and why?" → cited answer with section references. The landing page's primary asset.
- **App UI polish pass**: the React SPA is functional (B8 wizard, Ask/Sources/Review tabs); a design pass on visual hierarchy, empty states, spacing/typography so in-app screenshots used on the site look credible and the download CTA converts.
- **Download page assets**: clean app screenshots + checksums table for the GitHub Releases artifacts.

**DoD:** a logo + visual system exist and are applied; the landing page and in-app screenshots look intentional, not template-y; the 60s demo video is produced; B33 ships with B38 assets.

### B39. Scale hardening: search-backend seam + stage-0 fixes (P1 — code-review findings 2026-08-11)
**Problem:** the codebase is clean but has three scale assumptions that break at exactly the "point it at the company wiki" moment: O(N) pure-Python vector scan per query, O(n²) duplicate detection on a GET, and sync HTTP probes inside async endpoints that block the whole event loop while Ollama is down. None of them are visible at demo scale; all three are cheap to fix now and expensive to retrofit after a hosted tier exists.

**Scope:**
- **S0a — async status probes** (`status.py`): `ollama_reachable`/`missing_ollama_models` use sync `httpx.Client` inside async endpoints → dead Ollama blocks *every* request ~2–4s. Make them async (or add a short TTL cache keyed on base_url).
- **S0b — cap duplicate detection** (`routers/entities.py` review `duplicates`): restrict candidates to low-confidence + recency-capped entities (not all-vs-all); add a unique index on `merge_actions(entity_a_id, entity_b_id)`; move proposal generation off the GET (job or lazy render) so concurrent requests can't double-write.
- **S0c — native KNN**: `AnswerEngine._vector_search` unpacks every embedding blob into Python lists per query. Move the search into sqlite-vec `vec0` KNN (exact, in C/SIMD) behind a **`SearchBackend` seam** (`vector_search`/`keyword_search`/`who_knows`), keeping the pure-Python path as fallback for extension-less builds. Exact KNN in C is fine to ~1M chunks — no ANN needed for the local product.
- **S1 (deferred)** — local at 100k+ chunks: single asyncio writer queue if write contention shows up; embed cache keyed by `content_hash` (cost lever); FTS5 is fine as-is.
- **S2 (deferred, hosted only)** — track A: SQLite per-tenant + litestream→R2 (small teams); track B: Postgres + pgvector + object storage behind the same `SearchBackend` seam (multi-region/heavy writers). Ingestion workers split from the API process; `team_id` tenant boundary. See [v2v3-scope.md](./v2v3-scope.md).

**DoD:** `/health` never blocks other requests; duplicate review stays responsive past 10k entities; vector search runs natively in sqlite-vec with the seam in place so the hosted Postgres track becomes additive, not a rewrite. Telemetry hooks (B34) log chunk count + query latency to make the S1/S2 triggers measurable.

---

## Current execution priorities (agreed 2026-08-07)

Explicit order — the retrieval/answer architecture is the focus while tokens are cheap.
**B32, B17, B18 and B15 are DONE** (released in v1.0.3–v1.0.6); the queue below is what remains.
B30 is large and cross-cutting so it sits last in line but is **flagged for design input
before implementation** (see [B30 open question](#b30-data-labeling-piisensitive-gating-of-models-sharing-and-answers-p1-for-pii--v1-risk-p2-rest)).

> **B30 (needs product input on the PII taxonomy)**

| # | Item | Status | Why here |
|---|---|---|---|
| 1 | B32 Graph-based retrieval | ✅ DONE (v1.0.3) | graph walk after RRF → connected entities in context + citations |
| 2 | B17 Planner→Executor→Synthesis | ✅ DONE (v1.0.4) | tool planner + executor + evidence fusion; who_knows tool |
| 3 | B18 Distillation | ✅ DONE (v1.0.5) | normalized Q&A units + IDF-gated embedding |
| 4 | B15 Scoped search / projects | ✅ DONE (v1.0.6) | project bundles of sources; QA scoped via project picker |
| — | B19 newest answer on top | ✅ DONE (v1.0.7) | Ask renders newest-first |
| — | B3 Dispute tracking | ✅ DONE (v1.0.7) | audit trail + counter; disputed excluded from Q&A |
| — | B7 Source config editing | ✅ DONE (v1.0.7) | PUT /sources/{id} + edit form; keychain-backed secrets |
| — | B8 First-run wizard | ✅ DONE (v1.0.7) | onboarding overlay: Ollama checks → connect → ask |
| 5 | B30 Data labeling / PII gating | **next** | large + cross-cutting; **needs product input first** (see below) |
| 6 | B39 Scale hardening (S0: async probes, dup cap, native KNN + SearchBackend seam) | **new** | code-review findings 2026-08-11; small + high-leverage, unblocks hosted tracks |

**GTM tracks (decided 2026-08-10) — run in parallel with B30, not in its critical path:**

| # | Item | Status | Why here |
|---|---|---|---|
| 7 | B36 Commercial licensing (BSL 1.1 + key gate) | **new** | unblocks testers legally + on-prem sales; prerequisite for B37 |
| 8 | B33 Website + docs + changelog | **new** | findability; landing CTA → GitHub Releases |
| 9 | B34 Opt-in telemetry | **new** | validation signals before pricing is set |
| 10 | B35 Hosted tier + pricing (sub-only) | **new** | design locked; build when hosted pilot starts |
| 11 | B37 Enterprise on-prem platform | **new** | v3; sells the platform, not hosting |
| 12 | B38 Design & brand work | **new** | visual layer B33/demo/download page depend on |

*Companion docs: [architecture.md](./architecture.md), [packaging.md](./packaging.md), [mcp.md](./mcp.md), [rust-port.md](./rust-port.md)*
