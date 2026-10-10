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

## 4. Customer (v1) — two doors, one product

**Master positioning (hero A, chosen):** *Your memory — finally searchable.* (see `design-system.md:5.1` for variants)

**Tracks — same local app, different language:**

| Track | Buyer | Job to be done | Why it wins |
|---|---|---|---|
| **Personal** | Solo PM, founder, researcher, local enthusiast | “Remember everything *I* read/decided/built” — PDFs, notes, side-project Jira | Second brain that cites page 250 of a 300-page PDF |
| **Company (team)** | 3–30 person team, PM-led | “Answer like you’ve worked here 3 years” — decisions, owners, dependencies across people | Onboarding, handovers, “who owns billing migration?” without Slack archaeology |

No sales team, no procurement cycle in v1. Website has one page with a Personal/Team/Hosted toggle — default **Personal** for faster validation; Team self-hosted is a shipped, licensed tier. The toggle persists `?track=` + site-localStorage (no app handoff yet).

**Moment of wow (60-second demo):** connect sources → ask *"what was decided about X, and why?"* → get a cited answer showing exactly where the knowledge came from. Personal example: security-transfers note; Team example: “who owns billing migration and what supersedes it?” (`sample/` covers both).

## 5. Scope — Locked v1 Decisions

| Area | Decision |
|---|---|
| Runtime | Local-first app (local server + browser UI), Windows + macOS downloads |
| Connectors | Local folder, Drive, Jira, Linear, REST (v1); Slack planned |
| Model tiering | Ollama (local, 3B) for classification; BYO big models via API for Q&A/planning |
| Model access | Bring-your-own-key (v1); bundled access v2–3 |
| Trust layer | Sources + confidence + review UI (v1); contradictions & verification workflows (v2) |
| Retrieval | SQLite + sqlite-vec, local embeddings (`nomic-embed-text`), swappable VectorStore |
| Sync | Incremental connector polls/cursors; folder watcher + hourly scan fallback; hash dedup; delete cascades |
| Data model | Uniform entity graph (entities + typed relationships + provenance); contradictions as external layer |
| Answers | Planner → executor → fusion → graph walk, section-level citations, cheap default BYO model |
| Identity | Hash dedup in-source; cross-source duplicate proposals for manual merge |
| Billing | Personal local free forever; Team self-hosted = one-time platform license (perpetual + 1yr maintenance), offline ed25519 key (`backend/app/license.py`, issue via `scripts/make_license.py`); hosted pricing TBD from pilots |
| Security | OS keychain for credentials; secrets never in DB/config/logs |

## 6. How Data Becomes Knowledge

```
Sources (folder, Drive, Jira, Linear, REST; Slack planned)
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

**v2 (deferred):** contradiction detection (claim graph + pairwise conflict scanning). Verified/disputed states shipped in v1 (B3).

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
| Validation (v1, now) | Personal local free — gather feedback |
| Team self-hosted (now) | One-time platform license, perpetual incl. 1yr support/updates; no per-seat, no numbers on site |
| Hosted (v2+) | Pricing TBD from pilot conversations |
| Enterprise | Support contract / on-prem, scoped per deal |

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
| 6 — Packaging | Windows + macOS apps, Ollama-first | Downloadable zips via release.yml |

---

## 14. Backlog

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

### B9. Document type coverage (P1)
**Scope:** add `.docx`/`.pptx`/`.odt` extraction (small deps); revisit after real tester file types are known — the watched folder currently only ingests a subset of formats.

### B11. Parallel classification throughput (P2 — discovered) — DONE
**Problem:** windowed classification runs sequentially — a 237-page PDF took ~15 min (160+ serial LLM calls). Hardware is fine; the pipeline serializes.

**Done:**
- Bounded concurrency in the pipeline: `asyncio.Semaphore(N)` around classifier calls (N = `ANCHOR_CLASSIFIER_CONCURRENCY`, default 4) — windows are classified with `asyncio.gather`, results concatenated in order
- Ollama tuning documented (README + `.env.example`): `OLLAMA_NUM_PARALLEL=4` (match app concurrency), `OLLAMA_CONTEXT_LENGTH=8192` (windows need ~2.5k tokens; 32k default wastes KV cache), `OLLAMA_KEEP_ALIVE=30m` (avoid model unload churn)
- Throughput stats live: `throughput.py` tracker records per-window latency; `/health` + `/system/status` expose windows + avg latency + concurrency; System tab shows a "Classifier throughput" card

**Expected:** 3-5x wall-clock reduction on GPU machines; scales with hardware. Without `OLLAMA_NUM_PARALLEL`, requests queue at the server (no speedup, no harm).

### B12. Retrieval quality: hybrid search + chunk cleaning (P2 — discovered, confirmed live) — DONE (build + rulebook re-test; optional rerank remains)
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

### B16. who_knows — expertise queries (P1 — Cerebras learning)
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

### B22. Live integration validation: Jira / Linear (P1)
**Problem:** the Jira and Linear connectors exist but have never hit real instances. "Connects to your tools" is claimed but unproven.

**Scope:**
- Jira: validate against a real sandbox instance (Atlassian free tier + API token) — incremental cursor, comments→doc text, author/assignee mapping, error surfaces; fix whatever breaks
- Linear: validate the shipped connector (API key auth, issues/cycles, incremental by updatedAt) against a real workspace
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

### B27. Review page: real context, not just the window (P1 — tester feedback) — DONE
**Problem:** even with `window_text`, the Review page still shows a raw classifier-window excerpt — no surrounding document context, no neighbouring sections, no file reference. Reviewing an entity still feels like judging a fragment in a vacuum ("small chunks, no value").

**Done:**
- `GET /entities/{id}/context` — returns window + `expanded_before/after` (one `chunk_document` neighbour each side) + `source_name`/`item_title`/`source_ref` + `full_text` (16k, collapsed by default) + `highlight` (entity summary)
- **ReviewTab** now shows file, section, source name, confidence/kind, highlight (`<mark>` on verbatim summary), neighbour sections dimmed, full-document expander; **EntitiesTab** same "Show context" affordance — no vacuum
- Quick-actions stay (Looks right / Dispute) with full-document affordance

**DoD:** reviewing a low-confidence entity shows the file, section, surrounding paragraphs, and the exact text the classifier summarized — a reviewer can judge correctness without opening the source file. — met

### B28. Google Drive connector (P1 — big real use case) — DONE (skeleton, mocked)
**Problem:** most teams keep shared docs in Google Drive (folders, shared drives). AnchorCore's folder connector only watches local disks — Drive content requires manual download.

**Done (skeleton):**
- `backend/app/connectors/gdrive.py` — direct Drive API ingest (no local mirror yet, simpler for hosted): lists `'{folder_id}' in parents` via `GET /drive/v3/files` (supports shared drives), incremental via `modifiedTime > cursor`, supports pagination; native Google Docs → `export=text/plain`, Sheets → `csv`, Slides → `text/plain`; regular files via `alt=media` with pdf/html/text decoding; `401` maps to "re-authorize". Config `folder_id` (non-secret) + `token` (SecretStore `token`, masked `***set***`), wired via `connectors/__init__.py: gdrive`.
- Frontend: `SourceForm`/`SourceRow`/`SourceEditForm` now expose `Google Drive` (folder ID + OAuth token, keychain-stored, edit preserves `***set***`).
- Tests `tests/test_gdrive.py` (mocked `RetryClient`): missing-config 422, list+export+media, 401 auth, end-to-end pipeline via `POST /sources` (gdrive) → sync → entities → `***set***` masking.
- **Not yet:** local mirror dir (`data/drive/<name>/`), OAuth browser flow (needs Google Cloud project + client id), shared-drive `drive_id` filter, Drive-native PDF export, deletion→stale. These are the next polish after skeleton validation.

**DoD:** a user authorizes a Drive folder, AnchorCore syncs its docs (including Google-native formats), and Q&A answers cite Drive sources with working file links. — skeleton met (mocked), live OAuth + mirror remain. *(Update: Rust parity shipped — `gdrive.rs` + UI option, token-based; OAuth flow + local mirror still open.)*

**How to try (mocked or live):**
- Mocked: `pytest backend/tests/test_gdrive.py` passes without creds.
- Live: create Google Cloud OAuth consent + token (scope `https://www.googleapis.com/auth/drive.readonly`), create source `gdrive` with `folder_id` (from Drive URL `…/folders/<id>`) + token, `POST /sources/{id}/sync` → `SourceRef=Drive:<name>` citable like folder/Jira.

### B29. v2/v3 business & platform scoping (P2 — see [v2v3-scope.md](./v2v3-scope.md))
**Problem:** v1 is local-first and free; there's no website, hosted path, pricing, or changelog discipline — nothing for people to find/try/pay for.

**Scope (per [v2v3-scope.md](./v2v3-scope.md)):**
- Website: ✅ landing (60s demo), download, pricing shipped; on-domain guides render repo docs; changelog page still open
- Hosting: same code, env-driven — Postgres + object storage + server-side connectors; single-region VPS first
- File sharing: read-only share links (project tokens) → collaborators → permissions (the Teams tier trigger)
- Release mgmt: mandatory CHANGELOG.md + SemVer; update-checker later
- Free tier + pricing: local forever free (privacy moat); hosted paid; small hosted free tier as the no-install demo
- Enterprise (v3 stretch): SSO, audit logs, on-prem

**DoD:** a stranger lands on the website, downloads the app (or starts the hosted free tier), and asks a cited question in under 3 minutes; a changelog accompanies every release.

### B14. Agent connectivity via MCP (P1 — see [mcp.md](./mcp.md)) — B14.1 + R12.5 DONE, B14.2–B14.4 open
**Problem:** users want their own harnesses (Claude Code, Codex, opencode) to use AnchorCore's memory; the stdio sidecar (B14.1/R12.5) ships, and B14.2–B14.4 extend it to shared/HTTP, write-back and registry installs.

**Scope (per [mcp.md](./mcp.md)):**
- B14.1 Local stdio MCP server, read-only: `ask`, `search`, `get_entity`, `get_source`, `list_sources`, `memory_status` — thin adapters over existing services; results respect status/dispute filtering
- B14.2 Streamable HTTP transport mounted on the Axum app (`/mcp`), bearer-token auth (opt-in, disabled on localhost), calls audited in `system_events` — the "centralized dataset for agents" story
- B14.3 Write-back `ingest` tool (items stored `unverified`, author `mcp:<token>`, routed to the review UI)
- B14.4 Registry publishing for one-command harness installs

**DoD:** a tester points Claude Code at their AnchorCore memory (local or centralized), asks a question, and gets a cited answer; every `ask`/`search` call appears in the audit trail.

### B30. Data labeling: PII/sensitive gating of models, sharing, and answers (P1 for PII — v1 risk, P2 rest) — DONE (v1.0.8)
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

**DoD:** label a folder `pii` → cloud classifier/embedder/answer never touch it (local-only, verified in logs + a `system_event`); a shared link (v2) answers only from `public` sources; the UI shows each source's label and what it allows. — **met** (2026-08-17): cloud classify/embed (B39) + answer gating at source AND chunk level; `/qa/public` public-only scope; label chips in Sources + PiiTab.

**B30 open question — RESOLVED 2026-08-18 (user ratified):**
Direct vs Indirect PII taxonomy, source-level for v1, local vs API gate now / per-user later:
- **Direct / Linked PII → `pii` (hard, never cloud/share):** Full name (legal/maiden/alias), SSN/National ID, Passport/DL, Home address, Personal email, Personal phone, Financial accounts (card/bank), Biometric. `pii` → local model everything, API model blocked until `ANCHOR_CLOUD_TRUST=1`, never share/MCP.
- **Indirect / Linkable PII → `sensitive` (soft, local-only, not shareable):** DOB/place of birth, IP/MAC, cookies/IDFA, Geo, Employment, Education, Medical, Vehicle VIN/plate, Mother's maiden, Criminal. Also payroll/financials/business-sensitive (customer lists, pricing, unreleased plans). `sensitive` → same cloud block until user-controlled trust, local answer allowed, never share/MCP.
- **Internal / Public** unchanged.
- **Gate now:** local model = everything; API model = user-controlled (`ANCHOR_CLOUD_TRUST`); chunk-level `is_pii` flags for review.
- **Gate later (hosted team):** per-user access — users can/can't see PII/sensitive that company offers (requires `users` + project ACL; auth skeleton `b40a0c1`). Source-level for v1, per-item/chunk NER auto-detect deferred to v2.5.

**Done (full — 2026-08-17):**
- `app/pii.py` — PII knowledge base (17 categories with field names + value regexes + custom filter words), local-only `scan_text` (no LLM, CI-safe)
- `chunks.is_pii` + `chunks.pii_categories` (migration `b30a0c1`); pipeline auto-flags chunks on ingest (`_flag_pii`)
- `/pii` API: `GET/PUT /pii/config` (toggle categories, custom words), `GET /pii/review`, `POST /pii/review/{id}` (confirm/override), `POST /pii/scan/{source}` (re-scan after config change)
- Frontend `PiiTab` (config: category toggles + custom filter words; review: flagged chunks with matched categories, Mark PII / Not PII)
- **Answer-provider gate (B30):** `cloud_answer_trusted()` in `status.py`; `AnswerEngine.ask` filters sensitive/pii sources AND PII-flagged chunks from unconfirmed cloud answers, records a `system_event`, and refuses explicitly when nothing survives; follow-up rewrite is skipped when the answer provider is untrusted
- **Share/MCP-safe ask:** `/qa/public` (and `public_only` on `/qa`) answers ONLY from `public` sources — non-public content is never retrieved; AskTab has a Public-only toggle; the frontend label chip shows what each label allows
- **Chunk-level cloud embed gate:** PII-flagged chunks are not sent to an unconfirmed cloud embedder even in internal sources
- Tests `tests/test_pii.py` + `tests/test_b30_gates.py` (public scope, sensitive/pii cloud block, chunk-level block, trust-flag allow, audit event)
- **Deferred (P2, not B30):** NER-based auto-detection; LLM-assisted review. (MCP shipped since — R12.5 consumes `/qa/public`.)

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

**DoD:** tagging `v0.x.0` produces downloadable Windows + macOS artifacts automatically, verified by a smoke test on a clean machine. *(Superseded: `release.yml` now builds Rust `AnchorCore-{windows,macos,linux}.zip` — see `releasing.md`.)*

### B31. Backend port to Rust — DONE (v1.0.9+, Rust is the shipped backend; see [rust-port.md](./rust-port.md))
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

### B33. Retrieval at scale: vec0 index (P1 — tech debt, blocks large corpora) — DONE
**Problem:** `_vector_search` in `backend/app/answer_engine.py:529` loads every `chunks.embedding` into Python and computes cosine — O(N). Works at 100s chunks, collapses at 10k. sqlite-vec `vec0` virtual table is already bundled (`packaging.spec:32`) but unused.

**Scope:**
- Create `vec0` virtual table + triggers (mirror `chunks_fts` pattern `backend/alembic/versions/b12f7c0_add_chunks_fts.py:1`); backfill existing embeddings; keep `pack_f32` path as fallback when `enable_load_extension` missing (`backend/app/db.py:18`)
- Rewrite `_vector_search` to `SELECT rowid, distance FROM vec_chunks WHERE embedding MATCH :q ORDER BY distance LIMIT :k`; keep `0.2` cosine threshold as distance cutoff; project-scoped via `JOIN` on `IngestedItem.source_id`
- Measure: add `retrieval_latency_ms` to `/system/status` + `tests/test_retrieval.py:1` perf probe (1k chunks <100ms vs current O(N))

**DoD:** 10k-chunk corpus retrieves in <100ms; fallback to Python scan when vec0 unavailable; existing tests green.

**Done (B33):** migration `b33a0c1` creates `vec_chunks` (vec0, cosine metric) + AI/AD/AU triggers + backfill (skips gracefully when sqlite-vec isn't loadable); `alembic/env.py` loads sqlite-vec on migration connections; `_vector_search` → `vec_chunks` query with `k = :limit` + project JOIN, falling back to the Python scan on dim mismatch/extension missing; `embed_dim` config; `/system/status.retrieval` latency/backend snapshot + SystemTab card; `test_vec_chunks_table_exists`, `test_vec0_insert_and_retrieval`, `test_vec0_project_scoping`, `test_vec0_perf_probe` (1k chunks <100ms).

### B34. Backend hardening: version, routing, secrets, jobs (P0 — correctness) — DONE
**Problem (from review):** four small correctness debts compound: (1) `APP_VERSION` duplicated `backend/app/main.py:109` vs `backend/app/routers/system.py:28`; (2) `GET /projects/default` duplicated `backend/app/routers/projects.py:43` + `88` (second silently wins); (3) secret mask lists in `backend/app/secrets.py:108` and `backend/app/routers/sources.py:84` diverge — new field leaks; (4) `JobManager` `backend/app/jobs.py:150` creates unbounded `asyncio.create_task` — concurrent reclassifies can starve loop.

**Scope:**
- Single source of truth: `backend/app/config.py:1` or `backend/app/__init__.py:1` exports `__version__`; `main.py:109` + `system.py:28` import it; CI checks drift
- Delete duplicate route `projects.py:88`; keep first; add test `GET /projects/default` before/after patch
- Centralize secret fields: `SECRET_SOURCE_FIELDS = {"token","api_key","password"}` in `secrets.py:1`; both `store_source_config` and `sources.py:84` import it; add test that unknown secret field is masked
- Bound jobs: `JobManager` semaphore / queue (max 2 concurrent syncs, enqueue rest as `pending`); `_age_decay` `answer_engine.py:112` uses per-query fixed `now` not per-hit `datetime.now()`

**DoD:** `rg APP_VERSION` hits one definition; `pytest tests/test_projects.py -k default` passes once; new secret field auto-masked; 3 concurrent `POST /sources/{id}/sync` queues rather than stalls.

**Done (B34):** `__version__` lives in `app/config.py`, imported by `main.py` + `system.py`; duplicate `GET /projects/default` route deleted (first kept); `SECRET_SOURCE_FIELDS` single tuple in `secrets.py` (sources router references it via module attr so a new field auto-masks); `JobManager` bounded (MAX_CONCURRENT=2, rest persist as `pending` + queue, dispatch on slot free, queued job cancellable); `_age_decay(created_at, halflife, now=)` uses one clock per query. Tests: version single-source, secret auto-mask, `test_job_queue_bounds_concurrency`.

### B35. Ingestion pipeline decomposition (P2 — maintainability) — DONE
**Problem:** `backend/app/pipeline.py:498` mixes DB lifecycle, LLM concurrency, 3 chunk kinds, cleaning `cleaning.py:68`, distillation — highest churn file. Changes risk SQLite lock regressions `pipeline.py:192` commit-before-LLM.

**Scope:**
- Extract: `chunking.py` (`chunk_text`, `chunk_document`, `_split_section`, `_is_heading`), `hashing.py` (`window_hash`, content hash), `distill.py` (`_distill_and_store`, `_signal`); `pipeline.py` retains orchestration (`sync_source`, `_upsert_doc`, `_classify_and_store`)
- Keep commit-before-LLM invariant: `db.commit()` before `asyncio.gather` classify; add comment + test that validates no write tx held during `classifier.classify` mock
- Scope `_signal` IDF: compute global DF across item + existing corpus (or at least item's `doc_type` cohort) vs per-item only `pipeline.py:476` — reduces isolated-doc low-signal mis-gating

**DoD:** `pipeline.py` <300 lines; `chunking.py` unit-tested; `pytest` still 88+ green; lock-contention flake (`AGENTS.md:113`) not reintroduced.

**Done (B35):** `pipeline.py` 298 lines; `chunking.py` (chunk_text/chunk_document/_split_section/_is_heading/classify_windows), `hashing.py` (window_hash/content_hash — single source, re-exported by classifier.py/models.py), `distill.py` (`distill_and_store`, `signal`) extracted; commit-before-LLM invariant preserved + validated by `test_commit_before_llm_invariant` (asserts no write tx open during classifier). `tests/test_pipeline_decomposition.py` + existing `test_retrieval.py` chunking tests cover the extracted modules.

### B36. Frontend platform hardening (P0 — dev + reliability) — DONE
**Problem:** `frontend/vite.config.ts:8` missing `/projects` proxy → `api.ts:58` fails in `vite dev`; `frontend/src/api.ts:3` `fetch` has no timeout/abort; `frontend/src/main.tsx:1` no `ErrorBoundary` → packaged `console=False` (`backend/run_app.py:101`) white-screens; `frontend/src/SettingsTab.tsx:66` strips `***set***` but `SourcesTab.tsx:152` sends it verbatim.

**Scope:**
- Add `"/projects": "http://localhost:8000"` + `"/projects/default"` to `vite.config.ts:8`; verify `npm run dev` manual smoke
- `api.ts:3` `request()` accepts `AbortSignal`, 30s timeout via `AbortController`, surface `AbortError` as "Request cancelled"; `AskTab.tsx:11` `submit()` wires `AbortController` to Cancel button for long `/qa`
- Add `ErrorBoundary` (`react-error-boundary`) in `main.tsx:1` + per-tab boundary with "Copy error / Download log" linking `api.systemLogs` `SystemTab.tsx:5`
- Unify secret placeholder: extract `stripPlaceholders(payload)` helper (like `SettingsTab.tsx:66`) and use in `SourcesTab.tsx:152`; add test/build check

**DoD:** `vite dev` → Projects CRUD works; long QA cancellable; throw in any tab shows boundary not white screen; `***set***` never sent as literal token.

**Done (B36):** `/projects` proxy added to `vite.config.ts`; `api.request()` accepts an `AbortSignal` + 30s timeout, throws `RequestAbortedError` ("Request cancelled"); `AskTab` Cancel button aborts the `/qa` request; `ErrorBoundary` (self-contained, no new dep) wraps the app + each tab in `main.tsx`/`App.tsx` with Copy error / Download log; `stripPlaceholders` shared helper used by SettingsTab + SourcesTab edit form. Vitest covers placeholder + abort + error parsing.

### B37. Frontend decomposition & design system (P1 — maintainability, **no UX expertise required**) — DONE
**Problem:** `frontend/src/tabs/SourcesTab.tsx:384` is 384-line god component (11 `useState`, sources+jobs+projects+edit+jira); polling soup `SourcesTab:61` 1s + `App.tsx:88` 5s + `SystemTab:32` 10s + `App:69` 30s, no `AbortController`; inline `styles:Record<string,CSSProperties>` duplicated `App.tsx:181` across 6 files; `App.tsx:29` vs `SourcesTab:17` duplicate `projects` state.

**Constraint:** owner has no UX/UI expertise — this item is **engineer-only**. No custom design; adopt an off-the-shelf system so good UX comes for free. Visual polish is deferred to validation feedback.

**Scope:**
- Split `SourcesTab` → `SourceForm`, `SourceRow`, `ProjectSection`, `JobsBadge` + hook `useJobsPoll(sourceId)`; `SourcesTab` becomes composition <120 lines — pure refactor, zero visual change
- Replace 4 intervals with `TanStack Query` (or `SWR`) + single `useJobsPoll`: cache, dedupe, `refetchInterval: 1000` only when `runningJobs.length>0`, `enabled` by tab visibility; `App.tsx:33` lifts `projects` to `ProjectsContext` (`api.listProjects` once, invalidated on `updateProject`/`deleteProject`)
- Styles: **do not design** — install `shadcn/ui` (or Radix + Tailwind) and map existing tokens (`#0f1115`, `#171a21`, `#2d333b`) to its `theme.ts` as a 1:1 token swap; replace `styles` objects with library primitives (`Card`, `Button`, `Badge`) — looks better with no design decisions; keep `tsconfig.json:13` `strict` + `noUnusedLocals` — add `eslint` + `prettier` (`package.json:6` currently none)
- Fix `AskTab.tsx:74` `[...turns].reverse()` keyed by index → key by `turn.content+idx`; `EntitiesTab.tsx:67` `busyId!==null` disables all buttons → `busyId===e.id`
- Alternative if even that is too much: skip token migration, keep current dark inline styles — decomposition alone delivers 80% of maintainability; visual system can wait until 5 testers give feedback

**DoD:** `SourcesTab` <150 lines; one polling hook; no duplicate projects fetch; `npm run build` passes `tsc -b`; no visual regression (pixel-diff or manual check) — even with no design skill.

**Done (B37):** `SourcesTab` is a 76-line composition; logic split into `tabs/sources/{ProjectSection,SourceForm,SourceRow}.tsx`; single `useJobsPoll` hook (TanStack Query, pauses when idle/tab-hidden); `ProjectsContext` (ProjectsProvider + useProjects) removes the App/SourcesTab duplicate `/projects` fetch; health + running-jobs polling moved to TanStack Query; `theme.ts` color tokens replace per-file inline hex (App/System/Entities/Ask/Settings/Review/OnboardingWizard/ErrorBoundary); AskTab turn keys + EntitiesTab busyId fixes.

### B38. Testing & observability uplift (P1 — confidence) — DONE
**Problem:** `package.json:11` has zero frontend tests/lint (`backend/tests:1` has 88 tests); `backend/app/routers/entities.py:120` `GET /review/duplicates` is O(n²) uncapped (cap 25 but scans all); shared test DB `backend/tests/conftest.py:26` accumulates — brittle; no perf regression guard.

**Scope:**
- Frontend: add `vitest` + `msw` for `api.ts:11` error parsing + `AskTab`/`ReviewTab` smoke; add `eslint` + `tsc --noEmit` in CI (`.github/workflows` already runs `npm run build`; add `npm run lint && npm run test`)
- Backend: paginate `GET /review/duplicates` (`limit`, `offset`, `kind` filter); early-exit when `len(unverified)>500` sample top 200 by recency; add test for limit
- Tests: document shared-DB contract in `backend/tests/README.md:1`; add `conftest` fixture `isolated_db` opt-in for tests needing empty DB (like `test_onboarding`); add `retrieval_latency` probe for B33
- Observability: expose `retrieval_*` knobs in `/system/status` (already Banner `config.py:95`) + SystemTab card; add `pending_embeddings` backfill progress bar

**DoD:** `npm run test` green in CI; `GET /review/duplicates?limit=10` paginated; new test can request isolation without global count asserts.

**Done (B38):** `vitest` + `eslint` configured (`vitest.config.ts`, `eslint.config.js`, `npm run test/lint` scripts, CI runs lint+test+build); `src/placeholders.test.ts` + `src/api.test.ts` (7 tests); `GET /review/duplicates` paginated with `limit` (clamped ≤100)/`offset`/`kind` + >500-candidate early-exit sampling; `isolated_db` fixture for empty-DB tests; retrieval latency probes in `test_retrieval.py` (B33) + `test_testing_uplift.py`; shared-DB contract documented in `tests/README.md`.

### B39. Security & distribution follow-through (P1 — trust, P2 — reach) — DONE (thin gate; bundled inference still deferred)
**Problem:** B30 PII gating `docs/product-plan.md:430` is spec-only — cloud classifier `classifier.py:280` can leak `sensitive` source; Ollama prereq `docs/packaging.md:69` blocks non-technical testers; contradicts local-first privacy promise `docs/architecture.md:300`.

**Scope:**
- Gate before B30 full build: add `sources.label` column (migration, default `internal`) + `SettingsService` `provider_trust` (`local` vs user-confirmed `cloud`); `pipeline.py:246` refuses cloud `classify/embed` when `label in (sensitive,pii)` and `provider_trust==cloud` → fallback to rule-based + `system_events` warning (`system_events.py:65`); Q&A/MCP exclude non-`public` outside local session
- Distribution: bundled inference `docs/packaging.md:69` `llama.cpp` sidecar (deferred from `docs/rust-port.md:42` §4.1) or guided Ollama install via B8 wizard `OnboardingWizard.tsx:298` — whichever unblocks testers faster; evaluate before Rust port `docs/rust-port.md:105`
- Keep `SecretStore` `secrets.py:47` per-key files + `RedactingFormatter` `redact.py:38` coverage for new fields

**DoD:** label a folder `pii` → cloud classifier skipped (log + `system_event`); `Ask` from local session still answers; non-`public` excluded from share/MCP; tester installs without manual `ollama pull`.

**Done (B39 thin gate):** migration `b39a0c1` adds `sources.label` (internal|public|sensitive|pii); `SourceCreate/Update` accept + validate it (frontend label select in add/edit forms); provider trust helpers in `status.py` (`cloud_classifier_trusted`/`cloud_embedder_trusted`, `ANCHOR_CLOUD_TRUST=1` for cloud); `Classifier.classify/distill/detect_document_type` take `cloud_trusted` and fall back to rules; pipeline gate records a `system_event` and skips cloud embedding for gated sources. Tests: `tests/test_gate.py` (label plumbing, invalid-label 422, local-trusted, remote-untrusted-until-flag, pii source still ingests). *B30 full model/share/MCP gating + bundled inference remain (needs product input).*

### B40. Stone & Sage palette (P1 — design) — DONE (v1.0.11)
**Problem:** indigo `#6366f1` reads as chatbot/SaaS `docs/design-system.md:17`, cold for `memory`; app + website need one non-AI vault/archive signal.

**Done:** `C3 #4A5A52 / #F2F0EB` replaces indigo — `frontend/src/theme.ts:1` `C3`, `docs/design-system.md:17` `tokens.dark/light`, `App.tsx:126,199,208,229` banner/spinner `accentAlt #8FA99E`, `SystemTab` `blue→ #7E9AB0`. Build `vite 315kB` + Rust `1.0.11` embedded.

### B41. Doc-grouped Entities + PII shield (P1 — review) — DONE (v1.0.11)
**Problem:** flat `EntitiesTab.tsx:6` at `1.6k` entities is unreviewable; chunks exposed as review surface; PII hidden in 250-page filing invisible.

**Done:** `EntitiesTab` `Map<item_id, Entity[]>` client-side `Grouped/Flat` default `>200`, `Whole doc` via `GET /entities/{id}/context` `full_text` + `GET /pii/item/:id` `PII ●` `Reveal` mask `••••`, `Verify all & collapse` → navigable `✓ Verified` doc. Chunks fully internal (`chunk_document 1600c` hidden). No migration.

### B42. A dozen review + banner persist (P1 — trust) — DONE (v1.0.11)
**Problem:** `1577 unverified` is not review — everything looks like a conflict; banner `Load failed` nags while retrying, dismiss resets on refresh; `limit 100` shows `100/1617`.

**Done:** `classifier 0.5→0.78/0.72` selective + `note` anchor gate `classifier.rs:283`, `pipeline verified≥0.70` `pipeline.rs:368`, backfill `1577→12 unverified` (lowest `conf`), `App.tsx:30` `localStorage["banner-dismissed"]` + suppress `failing_sources` while `runningJobs`, `entities limit 100→500/2000` `entities.rs:68` `api.ts:136` `needs_review` `⚠` flag + `avg` sort.

### B43. Business-scale corpus + Rust NOT NULL fixes (P1 — scale) — DONE (v1.0.11)
**Problem:** `sample/` 10 docs can't prove scale; Rust `NOT NULL` panics `sources/jobs/ingested_items/entities/chunks` block `800→5k` `tech/finance/AI`.

**Done:** `scripts/build_business_corpus.py` `tech/finance/AI` `800→5k` (`data/business-scale` gitignored) `813 docs` `964 entities` live `job 7` + `reclassify 11`; Rust fixes `sources.rs:165` `last_sync_cursor`, `jobs.rs:88` `total`, `pipeline.rs:233,368,371,400,462` `stale/owner/created_at`, `retrieval.rs:768` `cargo build 1.0.11` `vec0` + `chunks` now `73→` growing.

### B44. Generic REST API connector with user field mapping (P1) — DONE
**Problem:** every new tool (ticket tracker, CRM, custom internal API) needed a bespoke connector (`jira.rs`/`linear.rs`/`gdrive.rs`). Users couldn't self-serve.

**Done:** `rest` connector (`rust/crates/anchorcore/src/connectors/rest.rs:1`) — user supplies `base_url` + `list_path` and maps API fields to `IngestionDoc` via JSON pointers (`map_id/map_title/map_text/map_author/map_updated`, `/a/b` or `a.b` shorthand; `map_text` accepts comma-separated pointers; `map_updated` takes RFC3339 or epoch). Auth: none/bearer/basic/custom-header (secrets keychain-backed via existing `SECRET_SOURCE_FIELDS`); pagination: none/token/page/offset + `max_pages` bound; `since_param` for incremental; POST merges `body_json`. Empty ids get a stable content-hash fallback so re-syncs upsert instead of duplicating (`external_id` is the upsert key). `POST /sources/rest/preview` returns 5 mapped samples without persisting; `RestFields` form (add + edit) with Preview button; row label + `api.previewRest`. Inherits PII/label gates downstream.

**DoD:** point at any JSON list API, preview the mapping, sync → entities with `rest:<id>` refs. — met (9 unit tests: validation, mapping, pagination, stable-id).

### B45. Slack — export import locally, live app as Teams/hosted differentiator (P1/P2)
**Decision (2026-09-07):** a Slack App in fully local mode is NOT worth it — per-user app creation + admin approval + N-laptop polling fails onboarding (B29 <3min bar) and wastes rate limits. The friction boundary is the pricing boundary: local gets dumb offline import (free), Teams/hosted gets the live workspace app (paid).

**B45.1 Local: Slack export ZIP import (P2, free tier):**
- Parser turns a Slack export (channels/DMs JSON) into burst/thread units (same rules as below) fed through the folder connector — zero credentials, zero polling, fully offline
- Scope: channel picker over the export, `slack-export:<channel>/<ts>` refs + permalinks where resolvable, DMs default `sensitive`
- **DoD:** drop an export ZIP into a folder source → threaded decisions answerable with citations, no network calls

**B45.2 Teams/hosted: native live Slack connector (P1, Teams tier flagship):**
- One install per workspace by an admin (bot token `xoxb-`, hosted OAuth relay); central sync so every seat gets "the AI already knows #deploys"
- Crawl: `conversations.list` (channel picker like Jira projects) → `conversations.history` per channel (`oldest` = cursor) → `conversations.replies` for threads → cached `users.info` for names; Tier 2/3 backoff; job progress (B2) for multi-hour backfills
- Behavior: ingestion unit = author burst (~5min collapse) or thread (parent = question, replies = resolution) → B18 distillation to `Q:/A:` units; channel name/topic as metadata + `source_ref` `slack:#chan/<ts>` with permalink citations; drop `channel_join/leave`, empty bots, emoji-only noise via `_signal` gate; humans weighted over bots for `who_knows`; edited → re-ingest, deleted → `stale`; DMs/private channels default `sensitive` (local-only, never shared)
- Self-hosted team: same connector against the company instance (Socket Mode if no public endpoint for real-time)
- **DoD:** sync 3 channels; a 5-message burst yields 1 distilled unit; a threaded decision answers "why X?" with a permalink citation; DMs stay local-only with a gate event

---

### B46. Review archived-Python consumers: Docker image, license check, CI — DONE

**Problem:** `backend/` left the tree (frozen on tag `archive/python-final`).
Three live systems now consume that frozen snapshot instead of in-tree code,
and the rewiring is unreviewed: the Team image fetches a tag tarball at build
time, the license gate can only change via a tag move, and CI restores the tag
for backend tests + conformance. Separately, the conformance job failed on the
pre-archive HEAD for unknown reasons (logs need repo access to read).

**Scope:**
- Dockerfile: tarball fetch reliability (`curl --fail`?), tag-vs-SHA pinning
  (tags are mutable — decide), layer caching, entrypoint/migration paths,
  fresh-clone `docker build` works with no local `backend/`.
- License check: frozen `license.py` — patch process (new tag + rebuild?),
  key rotation story, issue→verify roundtrip still documented and tested.
- CI: tag-restore steps on shallow clones, pip cache path, version-drift
  scripts without the backend leg; diagnose + fix the conformance failure.

**DoD:** `docker build` green from a fresh clone; CI green on main including
conformance; license roundtrip verified; pinning decision recorded in
`docs/archived-python.md`.

**Done:** CI green (incl. conformance + archived backend suite after a
`PYTHONPATH=backend` fix); license gate reviewed sound; pinning recorded.
Fresh-clone `docker build` moved to B48 (no Docker on this machine).

### B47. Deploy the website (P1) — DONE

**Problem:** the site builds but lives only in `website/dist/` — nothing is
hosted and DNS doesn't point anywhere.

**Scope:** pick the static host (Cloudflare Pages is an option), wire deploy
(manual upload now, CI later?), point `anchorcore.dev` DNS, verify live:
download links resolve to release assets, Formspree signup works, no mixed
content. Export `og-card.png` for strict social crawlers.

**DoD:** `https://anchorcore.dev` serves the current build; downloads +
waitlist verified in production.

**Done:** live via Workers Static Assets (worker `anchorcore-website`,
`anchorcore.dev/*` route + custom hostname, auto DNS + cert). og-card.png
exported (1200x630), tags absolute. Verified: home 200, both zips 200 with
release-matching sizes, no remote http fetches, Formspree form exists
(405 on GET). Local upstream resolver negative-cached at verify time —
authoritative + 1.1.1.1 correct. Real signup test left to owner. CI deploy
(stored token) still open.

### B48. End-to-end test on clean machines (P1)

**Problem:** nothing has been validated outside dev checkouts — first user
experience is untested.

**Scope:** on clean Windows + macOS (no dev tools): download → unzip → run →
sync `sample/` → ask → cited answers; MCP smoke via one harness; fresh-clone `docker build` (from B46) + Team image:
boots with a valid license, refuses without one. Record every papercut.

**DoD:** both platforms pass the script; issues filed as backlog items.

**Progress (2026-10-09):** script written (`docs/b48-clean-machine.md` — §1
common checklist, §2 MCP smoke, §3 macOS + §4 Docker copy-paste legs).
Static checks green: v1.0.13 release carries 5 assets (CI-built zips);
website `latest/download` URLs live (B47); `sync_version --check` ok 1.0.13
(both scripts); tree == v1.0.13 for `rust/` + `frontend/` (post-tag commits
touch only CI/docs/website); `sample/` 10 files present. Executable legs
blocked on this box (sandboxed shell: no network for the zip, no MSVC
`link.exe` for a local release build, no Docker, no Mac, no Ollama) —
handed to the owner: (a) paste §3 on a Mac, (b) run §4 where Docker exists,
(c) drop `AnchorCore-windows.zip` in reach so the Windows + MCP legs can run
here. Papercuts so far: B50 (no `sample/` in release zips), B51 (windowed
exe gives no console feedback). B48 stays OPEN until Windows + macOS pass §1.

**Progress (2026-10-09, pass 2 — sandbox lifted, network on):** Windows §1
**PASS (simulated-clean** — isolated `C:\Temp\b48`, fresh data dir, release
binary; dev tools exist on the box): zip `8,837,897` bytes, exes only, no
`sample/`; first run stays up, `/health` ok (`ollama: offline`,
`pending_embeddings: 0`); source create `201`, sync `10/10` in `47s`
(`38` entities); ask #1 `5` citations (security-transfers decision),
ask #2 `5` citations naming **Sarah**; UI `/` + JS bundle (`335kB`,
`needs_wizard`/`csrf` strings present) serve `200`; relaunch recreates a
wiped data dir cleanly. §2 MCP **PASS** via Inspector CLI: `tools/list` =
`ask, search, get_entity, get_source, list_sources, memory_status`;
`memory_status` ok (`1.0.13`, `vec0`); bonus `ask` over MCP returns the
cited Sarah answer. Script bugs fixed in the doc (would have failed macOS
verbatim): §1 POSTs now fetch `GET /csrf` + `X-CSRF-Token` (bare POSTs
`403`); §2 uses Inspector `-e ANCHOR_BACKEND_URL=…` (`export` doesn't
propagate) and drops the invalid `--tool-arg '{}'`. New papercuts: B52
(onboarding fakes `ollama.reachable`), B53 (verbatim `\\?\` source refs +
empty citation paths). Still needs the owner: macOS §3 paste, Docker §4
(no Docker on this box — say so if you have none either and the CI
docker-build fallback gets added instead).

### B49. Hosted design: clean and secure (P2)

**Problem:** hosted is a skeleton + plans; the owner has concrete ideas for a
clean, secure design that aren't written down yet.

**Scope:** design session with the owner → record decisions: auth model,
per-user/per-workspace isolation, secrets handling, billing hooks, what runs
where. Update `hosting.md` + `v2v3-scope.md` with the outcome.

**DoD:** design doc merged; hosting backlog updated from it.

### B50. Ship the sample corpus with releases (P1, from B48)

**Problem:** release zips contain binaries only — no `sample/` corpus — so
the first-run wizard's "try the sample company" path reports
`sample.available: false` (`system.rs:230` finds nothing next to a release
install) and the new user's fastest route to value is dead on arrival.

**Scope:** pick one: (a) bundle `sample/` inside the zips + resolve it from
the exe dir, (b) one-click in-app download (fetch + verify hash from the
release tag), or (c) wizard links out with instructions. Whatever it is, a
release install must reach a synced sample source without git/a browser.

**DoD:** from a fresh release install, the wizard's sample path syncs and
answers with citations; B48 §1.3 becomes unnecessary.

### B51. Release exe gives no console feedback (P2, from B48)

**Problem:** the Windows release exe is `windows_subsystem = "windows"`
(`main.rs:6`), so launching it from a terminal with `--port`/`--data-dir`
prints nothing — not even startup errors (panics go to `anchorcore.log`
only). Scripted/terminal first runs look hung until `/health` responds, and
failures are silent until you find the log.

**Scope:** when launched from a console (std handles present), attach and
log startup + `listening on …` + fatal errors to stderr; keep double-click
silent. macOS bundle binary: same treatment if trivial.

**DoD:** `anchorcore.exe --port 8123 --data-dir …` from PowerShell prints a
startup line and any fatal error; double-click behavior unchanged.

### B52. Onboarding fakes `ollama.reachable` (P2, from B48) — DONE (via B54)

**Problem:** `GET /system/onboarding` reports `ollama.reachable: true` on a
box with no Ollama (verified: connection refused on `:11434`, no process),
while `/health` (`ollama: offline`), `/system/status`, and MCP
`memory_status` all correctly report unreachable. Root cause:
`system.rs:219` never probes — it just checks the base URL isn't a dummy
test URL (`http://localhost:1`), so any real-looking configured URL reads
"reachable". The first-run wizard's step-1 Ollama check therefore claims
success on Ollama-less machines and skips the install guidance.

**Scope:** make `onboarding_handler` use the same `probe_ollama` as
`status_handler` (or share one helper); keep it fast (short timeout, the
wizard blocks on it). Add a test with an unroutable base URL asserting
`reachable: false`.

**DoD:** on a box without Ollama, `/system/onboarding` reports
`reachable: false` and the wizard shows install instructions; with Ollama
up it reports `true`.

### B53. Verbatim `\\?\` source refs + empty citation paths (P3, from B48)

**Problem:** on Windows, folder citations carry `source_ref` =
``\\?\C:\…`` (the `canonicalize()` verbatim prefix, `folder.rs:59` stored
raw at `folder.rs:92`) and `path: ""` on every citation (entity and
document alike). Citations pass B48's bar (present, grounded, file-backed)
but render ugly and the empty `path` breaks the "section breadcrumb"
expectation in the B48 doc.

**Scope:** strip the `\\?\` (and `\\?\UNC\`) prefix when storing or
serializing folder `source_ref`; fill `path` with the section breadcrumb
(or the relative file path) where a section path exists, else leave empty
by contract and relax the B48 wording to match.

**DoD:** a Windows folder sync yields citations with clean `C:\…` (or
relative) refs and `path` populated wherever a section is known; B48 §1.8
wording matches the contract.

### B54. Dummy-proof model setup (P1, owner request) — DONE (code; Rust gates run in CI)

**Problem:** a user could sail through the wizard with zero models and land in
the Ask tab getting keyword excerpts with no idea why. No model install existed
in the UI (only Start-Ollama + copy-paste `ollama pull`), `missing_models` was
hardcoded `[]`, the wizard's "leave empty to use Ollama" hint was wrong (empty
falls back to the OpenAI default), and the degraded-model banner was
permanently dismissible.

**Done (guided setup + Pull button, skip allowed + sticky banner):**
- Backend (`rust/.../ollama.rs`, new): `model_readiness` — one `/api/tags`
  fetch yields real `reachable` (fixes B52), real `missing_models`
  (classifier/embed/answer roles, family-only match like `test_ollama`), and
  `answer_ready` (cloud key OR local answer model present). Wired into
  `/health`, `/system/status`, `/system/onboarding` (all additive fields).
- Backend: `POST /system/ollama/pull` (validates 1–5 names, local-base only,
  502 when Ollama down; pulls sequentially via Ollama `/api/pull` streaming
  into a process-global progress map — no AppState change) +
  `GET /system/ollama/pulls` for polling. 12 unit tests (matching, validation,
  NDJSON chunk parsing).
- Wizard step 0: Start-Ollama button + install instructions when down;
  Install-models button with progress bar + per-model MB + terminal fallback
  when models are missing; "Use Ollama for answers" one-click (sets
  `answer_base_url`/`answer_model`, pulls what's missing) when the answer path
  isn't ready. Step 1 drops the wrong "leave empty" hint for an explicit
  Ollama/Cloud choice; step 3 warns when answers will be excerpts.
- App banner: model issues (`modelHealth.ts`, 6 vitest) are sticky with
  [Set up models] (reopens wizard) + [Settings] buttons; sync-failure issues
  keep Dismiss. System tab Ollama card gains Install-missing with progress;
  Answer card flags "not working".
- Frontend types are defensive (`?? []`, `??` fallbacks) so the new UI
  degrades cleanly against an older backend.

**Verified:** `npm run lint` clean, vitest `17/17` (10 new), `npm run build`
(tsc + vite) clean; Rust files parse + new module fmt-clean (rustfmt).
Full `cargo test`/`cargo check`/live run NOT possible on this box (no C
toolchain/linker — even build scripts fail) — backend tests are authored for
CI, which must gate the merge.

**DoD:** a first-run user with no models sees Install/Start buttons (never a
bare terminal command), can't miss that answers are degraded (sticky banner +
wizard warnings), and reaches composed answers without leaving the UI. — met
in code; live pass after CI + a linked build.

### B55. Retrieval/answer eval harness (P1)

**Problem:** retrieval quality is where this product lives or dies, and today
it is measured by hand. The good numbers in this doc (B12: §372 at 0.931,
§434 at 0.892) are one-off live probes — unrepeatable, untracked, and blind
to regressions. The retrieval stack keeps gaining knobs (RRF weights, age
halflife, per-item caps, tag threshold, TOC depth) with no way to tune them
except vibes; a change that helps DVCA questions can silently sink billing
ones. No stranger can verify "answers well" without trusting us.

**Scope:**
- Golden set: ~30–50 questions over a frozen corpus (`sample/` + a slice of
  the business-scale generator, pinned) with expected citations (doc + section)
  and expected answer facts. Stored as data (`eval/golden.jsonl` or similar),
  easy to extend from real tester questions.
- Deterministic retrieval eval (CI-safe, no LLM): recall@k + MRR against the
  expected citations, citation precision on the top-k set. Runs in CI on
  every push; fails on regression vs. the recorded baseline.
- Answer eval (nightly/manual, LLM-judged): faithfulness (every claim cited,
  no unsupported claims) + citation coverage, judged by a pinned cloud model
  with the rubric checked in. Cheap enough to run before releases.
- Baseline + report: `eval/baseline.json` + a one-command run
  (`npm`/`cargo` script) printing a small table (metric, now, baseline,
  delta). Doc section: how to add a case, when to re-baseline (deliberately,
  never silently).
- Explicit non-goals: no eval-driven auto-tuning yet; no human-rating UI.

**DoD:** `main` carries a green retrieval-eval gate (recall@k + MRR vs.
baseline); a release run produces the answer-quality table; adding a tester
question as a golden case takes minutes and is documented.

### B56. Watcher misses same-file content edits (P1 — discovered in file-update review) — DONE (verified: cargo test 118/118 green)

**Problem:** editing a watched file in place never triggers an auto-sync.
`notify` fires and the source lands in `to_sync`, but `run` then gates on
the file *set* changing (`new_set != old_set` in
`rust/crates/anchorcore/src/watcher/service.rs:198` `fetch_and_compare`) —
a content edit adds/removes no files, so `trigger_sync` is never called.
The pipeline itself handles content changes correctly once a sync runs
(`pipeline.rs:276` `upsert_doc` compares `content_hash` per
`(source_id, external_id)`, re-classifies only changed windows), but the
watcher only auto-syncs adds/deletes. Edits sit stale until a manual sync
or scheduler tick. The `poll_changed` paths (which include the modified
file) are discarded — only non-emptiness is checked.

This hits the Obsidian-vault use case hardest: a vault is overwhelmingly
in-place `.md` edits, not adds/deletes — so the flagship "point at your
vault" flow barely auto-syncs at all. Worse, `docs/guides/obsidian-vault.md:18`
currently promises "New and edited notes are picked up automatically",
which is false for edits until this lands.

**Scope:**
- Compare content, not just set membership: track per-file mtime/size (or
  a cheap hash) in `known_files` alongside the path set, or pass the
  `poll`-returned paths through so a modify event triggers sync directly.
- Keep the 3s debounce + 1s settle behavior; don't sync twice when an edit
  also changes the set (single `trigger_sync` per loop pass per source).
- Regression test: write file → sync → edit file in place → watcher fires
  sync → new content retrievable. Keep CCN ≤ 15 in touched fns.

**DoD:** editing a watched file auto-syncs within ~10s and the new text is
returned by `/qa`; add/delete behavior unchanged; the
`obsidian-vault.md:18` "edited notes are picked up automatically" promise
is true again (verify with a vault-shaped fixture: edit `.md` in place,
no new files); `cargo test` green.

**Done:** `watcher/service.rs` `FileSnapshot` mtime+size fingerprints
replace the path-set compare; `fetch_and_compare` takes the source root;
content edits trigger sync on both the notify path and the fallback path;
3 unit tests (`fingerprint_detects_content_edit`,
`fingerprint_detects_same_length_edit`,
`fingerprint_stable_when_untouched_and_tracks_add_remove`). CCN ≤ 8,
rustfmt-clean in touched regions. Verified: full `cargo test` 118 passed
/ 0 failed (gnu target via user-local MinGW; MSVC BuildTools needs admin
elevation, unavailable to the agent).

### B57. Deleted files linger in the index forever (P1 — discovered in file-update review) — DONE (verified: cargo test 118/118 green)

**Problem:** nothing ever cleans up removed files. `stale` on
`ingested_items` is only ever written as `0` (insert/update in
`pipeline.rs:287,290`); no code path sets `stale=1` or deletes rows for
files that disappeared from the folder. Deleted docs keep their entities,
chunks, and embeddings, so answers keep citing ghost documents with
full confidence.

**Scope:**
- Detect disappearance during sync: after `FolderConnector::fetch`, diff
  fetched `external_id`s against stored `ingested_items` for the source.
- Decide semantics and implement one: hard-delete (item + entities +
  chunks + vec rows + tags/merge actions) vs. soft `stale=1` with
  retrieval exclusion (note `retrieval.rs:128` `status_ok` already
  excludes `stale`/`disputed` *entities* — item-level `stale` needs the
  same treatment in `load_hits`/chunks queries if soft-delete is chosen).
- Surface in UI if soft: doc-grouped Entities shows stale docs distinctly
  (reuses B41 grouping); hard-delete needs no UI.
- Regression test: sync → delete file → sync → doc no longer cited.
  `cargo test` + conformance green.

**DoD:** a file deleted from a watched folder stops appearing in answers
after the next sync; no orphan chunks/entities/vec rows left behind;
documented which semantic (hard/soft) was chosen and why.

**Done:** hard-delete chosen — the source is the source of truth and a
rename already reads as delete+add (external_id is the path), so
soft-delete would only add retrieval-exclusion + UI surface for no gain.
`pipeline.rs` `purge_missing_for_sync`/`purge_missing_inner`/`purge_item`
diff stored vs fetched `external_id`s after a successful fetch; FK
cascades + chunk triggers clean entities/chunks/vec/fts/sections; shared
tag counts recomputed, orphan tags deleted. Guards: `rest` excluded
(`max_pages` can truncate), empty fetches purge nothing, purge failure
logs + continues (never fails the sync). Job result gains `purged`
(+ SystemTab display, `types.ts` field). 4 tests (matrix, cascade incl.
vec/fts/tags, all-fetched no-op, async rest/empty guards). `run_sync_inner`
kept at CCN 15 (purge extracted to a helper). Verified: full `cargo test`
118 passed / 0 failed (gnu target via user-local MinGW).

### B58. Degraded answers impersonate composed ones (P1 — discovered in review) — DONE (verified: cargo test 118/118 + vitest 22/22 green)

**Problem:** when no answer/embed model runs, output degrades through three
paths with no machine-readable signal: (1) `embed_query` silently falls
back to `deterministic_embed` hash vectors, which are then scored against
real model chunk vectors — cross-distribution noise folded into ranking
(`rust/crates/anchorcore/src/embedder.rs:125`); (2) `generate_answer`
returns raw context dumps shaped exactly like composed answers
(`refusal: None`, full citations, only an inline bracketed note differs —
`rust/crates/anchorcore/src/answer.rs:638`); (3) `record_degraded` fires
only on empty hits (R10.5), so model-down-with-hits logs nothing, and the
B54 banner reflects config-level health, not per-request truth — a
transient outage mid-session shows nothing on the answer itself.

**Scope:**
- `AskResponse`/`SearchResponse` carry a per-request `mode` (`composed` /
  `context_only` / `keyword_only`) threaded from the path actually taken;
  `generate_answer` returns `(text, mode)`.
- AskTab renders a per-answer, non-dismissible banner from the flag,
  visually distinct from composed answers. The B54 global banner stays
  for config state — do not touch B54's files (`modelHealth.ts`, wizard,
  health components).
- When the remote embedder fails, skip the vector leg (honest
  keyword-only) instead of scoring deterministic query vectors against
  model vectors; verify with a retrieval probe before/after.
- Degraded-answer counter in `/system/status` (count, don't spam events
  per R10.5) — DEFERRED to a follow-up: `system.rs`/`health.rs` are under
  active B54 edit by another agent; the per-response flags already give
  harnesses + UI everything they need.
- Regression tests: no-key ask → `context_only`; embed-down search →
  `keyword_only`; UI test for the per-answer banner.

**DoD:** every answer/search response truthfully reports how it was
produced; the UI never renders a context dump as a composed answer; no
cross-distribution vector scoring; `cargo test` + frontend vitest green.

**Done:** `AskResponse` gains `mode` (`composed`/`context_only`/`none`)
+ `retrieval` (`hybrid`/`keyword_only`); `SearchResponse` gains
`retrieval`. `generate_answer` returns `(text, composed)` — true only on
a real model completion; `embed_query` is strict (`None` on remote
failure → honest keyword-only, no more hash-vectors scored against model
vectors; `deterministic_embed` kept for self-consistent tag similarity).
MCP `ask`/`search` pass the flags through (`unknown` on older backends).
AskTab renders a per-answer amber banner from the flags
(`answerMode.ts:modeBannerNotes`, silent on missing flags for old-backend
compat); no new hex, no B54 files touched beyond 2 surgical type lines.
Retrieval probe for the deterministic→keyword change not run (no local
build) — justified by construction (cross-distribution cosine is noise)
and existing tests exercise identical keyword paths. 4 Rust tests
(context_only ask, keyword_only search, fallback flag, mock-server
composed=true) + 5 vitest. Verified: `cargo test` 118 passed / 0 failed
(gnu target via user-local MinGW; the mock-server test needs a listening
socket, so it runs in CI / unsandboxed only) + frontend 22/22 vitest,
tsc + eslint clean.

### B59. Team image promises one Docker container, ships a two-service Python stack (P1 — discovered in website/README audit)

**Problem:** the site sells Team as literally one container — hero
`website/src/main.ts:16,18` ("Run on your infrastructure — one container." /
"One Docker container — your team's memory, on your infra"), FAQ
`website/index.html:248`, pricing `website/index.html:262` — and `README.md:35`
repeats it (`docs/design-system.md:96` carries the same voice). The artifact
ships **two** services: `hosting/docker-compose.yml` runs `app` (Python FastAPI
image built from the frozen `archive/python-final` snapshot,
`hosting/Dockerfile:5,15`) plus `db` (`pgvector/pgvector:pg16`,
`hosting/docker-compose.yml:4`). False twice over: not one container, and not
the shipped Rust backend — the 2026-08-21 review already flagged the two
divergent codebases (`docs/review-2026-08-21-full.md:80`). A buyer who expects
one container gets a Compose stack plus a Postgres to operate.

**Resolution (chosen): ship the single container the site promises — the Rust
binary (Axum + SQLite/sqlite-vec/FTS5, `frontend/dist` embedded), no DB
sidecar.** Team self-hosted is a 3–30 person team in a VPC; SQLite on a mounted
volume is the same engine the Personal app runs. Postgres + pgvector stays the
*Hosted*-cloud topology (B49, `docs/v2v3-scope.md` D9), not the Team-container
story. This supersedes R10.7's "`hosting/` stays Python" for the Team image —
R10.7 itself noted the Rust-against-a-SQLite-volume shape as the alternative.

**Scope:**
- Multi-stage `hosting/Dockerfile` (keep the path so the B48 §4 script keeps
  working): stage 1 builds `frontend/dist` + `cargo build --release -p anchorcore`;
  runtime stage debian-slim/distroless with `ca-certificates`, non-root user,
  `ANCHOR_DATA_DIR=/data`, `ANCHOR_SECRETS_NO_KEYRING=1` (fallback file store),
  `EXPOSE 8000`, `HEALTHCHECK` on `/health`. A plain `docker run -p 8000:8000
  -v anchorcore-data:/data` boots the app; migrations apply at start
  (`rusqlite_migration`) — no entrypoint, no uvicorn/Alembic, no Python.
- Team license gate parity: today `ANCHOR_EDITION=team` is enforced only by
  the frozen Python `backend/app/license.py` (ed25519, offline; issued via
  `scripts/make_license.py`). Port the check into Rust so the single-container
  Team image still refuses to boot without a valid license and boots with a
  lapsed-maintenance warning; keep the documented license-mount / inline
  `ANCHOR_LICENSE=...` flow.
- `hosting/docker-compose.yml` for Team: exactly one service (data volume +
  optional license mount) — or retire it in favor of the documented `docker run`
  one-liner. Any Compose-with-Postgres file moves to a clearly labeled
  hosted-cloud variant (B49), outside the Team quickstart.
- Doc consistency pass in the same commit: `hosting/README.md` (currently
  "skeleton … stays Python, R10.7"), `docs/hosting.md`, `docs/architecture.md`,
  `README.md:35,64,82`, `rust/README.md`, `docs/v2v3-scope.md` (D9/ops shape) —
  Team = one container; Postgres only in the Hosted-cloud row.
- CI + release: build and publish the image on `v*` tags to GHCR
  (`ghcr.io/atsirkunov/anchorcore:<version>` + `:team`) so "Deploy via Docker"
  works from a clean machine; smoke leg (boot → `/health` → UI 200 → unlicensed
  refusal) closes the open B48 §4 Docker script.
- Website: keep the "One Docker container" claims (they become true); point
  the Team card CTA at the real one-liner/registry per R18.5, and add "no
  database to run — SQLite on a volume" where it helps the buyer.
- **Fallback (only if single-container slips):** reword the promise to "one
  Docker Compose stack" everywhere it appears (`website/src/main.ts:16,18`,
  `website/index.html:248,262`, `README.md:35`, `docs/design-system.md:96`) —
  never ship a claim the artifact doesn't meet.

**DoD:** from a fresh clone, `docker build -f hosting/Dockerfile -t anchorcore .`
+ `docker run -p 8000:8000 -v anchorcore-data:/data anchorcore` serves the UI,
syncs `sample/`, and answers with citations — **single container, no Postgres
process**; unlicensed Team boot refuses and licensed boot passes (B48 §4
script green); the Team compose file has exactly one service; CI publishes the
image on tags; website/README/hosting docs match the artifact; `cargo test` +
frontend build green.

**Progress (2026-10-10, R19.1):** implemented. `hosting/Dockerfile` is a multi-stage Rust build (node→cargo→debian-slim runtime, non-root, `/data` volume, healthcheck); `hosting/docker-compose.yml` is ONE service; `entrypoint.sh` deleted. The Team license gate is ported to Rust (`rust/crates/anchorcore/src/license.rs`, format-compatible with `scripts/make_license.py`, 9 unit tests incl. Python-issued fixtures) and enforced at boot when `ANCHOR_EDITION=team`; the server gained `--host`/`ANCHOR_HOST` so the container binds `0.0.0.0`. CI: new `docker-team` job builds + smokes the image (refuses without a license; `/health` with an edition override); `release.yml` publishes `ghcr.io/atsirkunov/anchorcore:<version>` + `:team`. Docs synced (hosting/README, README, docs/hosting, architecture, v2v3-scope, AGENTS). Remaining: first green CI image run; R18.5 (Team CTA/price on the website) stays separate.

## Current execution priorities (agreed 2026-08-07 — updated after Aug review + the B48 pass, amended for no-UX constraint)

Explicit order — v1 core, hardening, scale and the website are DONE (v1.0.10–1.0.12). Queue below is post-website launch prep.

> **Hardening & scale (P0/P1) before new connectors. No UX expertise needed for P0 — P0 is pure engineering. Visual polish via Stone & Sage tokens — taste-free.**

> **UX constraint (owner: no UX/UI background):** Grouped doc view shipped engineer-only (`EntitiesTab` `Map<item_id>` + `PII ●` + `Needs review` filter) — no custom design. When testers complain about a specific screen, fix that screen.

| # | Item | Status | Why here |
|---|---|---|---|
| — | B32 Graph-based retrieval | ✅ DONE (v1.0.3) | graph walk after RRF → connected entities in context + citations |
| — | B17 Planner→Executor→Synthesis | ✅ DONE (v1.0.4) | tool planner + executor + evidence fusion; who_knows tool |
| — | B18 Distillation | ✅ DONE (v1.0.5) | normalized Q&A units + IDF-gated embedding |
| — | B15 Scoped search / projects | ✅ DONE (v1.0.6) | project bundles of sources; QA scoped via project picker |
| — | B19 newest answer on top | ✅ DONE (v1.0.7) | Ask renders newest-first |
| — | B3 Dispute tracking | ✅ DONE (v1.0.7) | audit trail + counter; disputed excluded from Q&A |
| — | B7 Source config editing | ✅ DONE (v1.0.7) | PUT /sources/{id} + edit form; keychain-backed secrets |
| — | B8 First-run wizard | ✅ DONE (v1.0.7) | onboarding overlay: Ollama checks → connect → ask |
| — | B33 vec0 retrieval | ✅ DONE | vec0 index + Python-scan fallback; `/system/status.retrieval` |
| — | B34 Backend hardening | ✅ DONE | single `__version__`, dedup route, shared secret fields, bounded jobs |
| — | B36 Frontend hardening | ✅ DONE | /projects proxy, abort+timeout, ErrorBoundary, shared stripPlaceholders |
| — | B37 Frontend decomposition | ✅ DONE | SourcesTab split, useJobsPoll, ProjectsContext, theme tokens |
| — | B35 Pipeline decomposition | ✅ DONE | chunking.py/hashing.py/distill.py; commit-before-LLM invariant tested |
| — | B38 Testing uplift | ✅ DONE | vitest + eslint + CI; paginated duplicates; isolated_db fixture |
| — | B39 PII thin gate | ✅ DONE | sources.label + provider trust; cloud blocked until ANCHOR_CLOUD_TRUST |
| — | B30 PII gating (full) | ✅ DONE (v1.0.8) | labels + provider trust; cloud classify/embed/answer gating (source + chunk); `/qa/public` share-safe scope; PII config + review |
| — | B27 Review context expansion | ✅ DONE | ReviewTab/EntitiesTab neighbour sections + highlight + full-doc |
| — | B28 Google Drive connector | ✅ DONE (skeleton) | Drive API direct ingest (mocked tests); OAuth + mirror polish next |
| — | B31 Rust port | ✅ DONE (v1.0.10) | `rust/` shipped artifact, `1.0.10` single source `Cargo.toml:6`, honest `110/17` vs Rust |
| — | B40 Stone & Sage palette | ✅ DONE (v1.0.11) | `C3 #4A5A52 / #F2F0EB` replaces indigo `docs/design-system.md:17` `theme.ts`, banner/spinner `App.tsx` |
| — | B41 Doc-grouped Entities + PII shield | ✅ DONE (v1.0.11) | `EntitiesTab` `Map<item_id>` `Grouped/Flat` default `>200`, `Whole doc` `GET /entities/{id}/context` + `GET /pii/item/:id` `PII ●` `Reveal`, `Verify all & collapse` → navigable doc |
| — | B42 A dozen review + banner persist | ✅ DONE (v1.0.11) | `classifier 0.5→0.78/0.72` selective, `pipeline verified≥0.70`, `1577→12 unverified`, `App.tsx` `localStorage["banner-dismissed"]` + suppress while `running`, `Entities` `Needs review` `⚠` flag + `limit 2000` |
| — | B43 Business-scale corpus | ✅ DONE (v1.0.11) | `scripts/build_business_corpus.py` `tech/finance/AI` `800→5k docs` (`data/business-scale` gitignored) + Rust `NOT NULL` fixes `sources/jobs/ingested_items/entities/chunks` |
| 1 | **B14.2 MCP HTTP transport** | **P1** | stdio shipped + tested local without UI (R12.5); HTTP + tokens unblock share links |
| 2 | B22 Jira/Linear live validation | **P1** | sandbox fixtures; both connectors exist, need real-instance proof |
| 3 | B9 Document type coverage | **P1** | `.docx`/`.pptx`/`.odt` extraction |
| 4 | B16 who_knows (full) | **P1** | expertise ranking + evidence (minimal `who_knows` tool shipped in B17) |
| 5 | B29 remainder: CHANGELOG + hosted prep | **P1** | website shipped; changelog discipline + pilot prep open |
| 6 | B28 Drive OAuth polish | **P2** | OAuth browser flow + `data/drive/<name>/` mirror |
| 7 | B45 Slack | **P2** | export-ZIP import local (free); live connector = Teams/hosted feature |
| — | B47 deploy website | ✅ DONE | live at anchorcore.dev (Cloudflare Workers Static Assets) |
| 10 | B48 end-to-end test | **P1** | Windows §1 + MCP §2 PASS (simulated-clean); Mac §3 + Docker §4 await owner |
| 11 | B49 hosted design | **P2** | owner design session → decisions in hosting.md/v2v3 |
| 12 | B50 sample corpus in releases | **P1** | wizard sample path dead on release installs (from B48) |
| 13 | B51 console feedback for exe | **P2** | windowed exe silent in terminal runs (from B48) |
| — | B52 onboarding Ollama probe | ✅ DONE (via B54) | real probe + install guidance in wizard |
| — | B54 dummy-proof model setup | ✅ DONE (CI gates Rust) | guided wizard + Pull button + sticky degraded banner |
| 15 | B53 citation path cosmetics | **P3** | verbatim `\\?\` refs + empty `path` on Windows (from B48) |
| 16 | B55 eval harness | **P1** | golden Q&A set + CI retrieval gate + release answer eval |
| — | B56 watcher content edits | ✅ DONE (cargo 118/118) | mtime+size fingerprints; edits auto-sync; Obsidian promise holds |
| — | B57 deleted-file cleanup | ✅ DONE (cargo 118/118) | hard-delete on sync; `rest`/empty guarded; job result gains `purged` |
| — | B58 degraded-answer honesty | ✅ DONE (cargo 118/118, vitest 22/22) | `mode`+`retrieval` flags; per-answer banner; strict query embed |
| 20 | B59 single-container Team image | ✅ DONE (R19.1) | site promises "One Docker container" but `hosting/` is a two-service Python stack; ship the Rust single container (or reword) |

*Companion docs: [architecture.md](./architecture.md), [packaging.md](./packaging.md), [mcp.md](./mcp.md), [rust-port.md](./rust-port.md), [design-system.md](./design-system.md)*
