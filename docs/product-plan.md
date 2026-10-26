# AnchorCore — Product Plan (v1.0)

> Status: plan — the living product/backlog narrative (canonical for scope); current engineering status is in `rust/BACKLOG.md`.

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

**Master positioning (hero A, chosen):** *Your memory — finally searchable.* *(Superseded 2026-10-21 by R44.9: hero is now "Ask your files. Get the answer and the exact source.", category noun "private AI search" — see `design-system.md` §5.)*

**Tracks — same local app, different language:**

| Track | Buyer | Job to be done | Why it wins |
|---|---|---|---|
| **Personal** | Solo PM, founder, researcher, local enthusiast | “Remember everything *I* read/decided/built” — PDFs, notes, side-project Jira | Second brain that cites page 250 of a 300-page PDF |
| **Company (team)** | 3–30 person team, PM-led | “Answer like you’ve worked here 3 years” — decisions, owners, dependencies across people | Onboarding, handovers, “who owns billing migration?” without Slack archaeology |

No sales team, no procurement cycle in v1. Website has one page with a Personal/Team/Hosted toggle — default **Personal** for faster validation; Team self-hosted is a shipped, licensed tier. The toggle persists `?track=` + site-localStorage (no app handoff yet).

**Moment of wow (60-second demo):** connect sources → ask *"what was decided about X, and why?"* → get a cited answer showing exactly where the knowledge came from. Personal example: instant-payments note; Team example: “who owns billing migration and what supersedes it?” (`sample/` covers both).

## 5. Scope — Locked v1 Decisions

| Area | Decision |
|---|---|
| Runtime | Local-first app (local server + browser UI); Windows + macOS downloads, Linux zip in CI |
| Connectors | Local folder, Drive, Jira, Linear, REST (v1); Slack planned |
| Model tiering | Ollama (local, 3B) for classification; BYO big models via API for Q&A/planning |
| Model access | Bring-your-own-key (v1); bundled access v2–3 |
| Trust layer | Sources + confidence + review UI (v1); contradictions & verification workflows (v2) |
| Retrieval | SQLite (FTS5 + in-code embedding scan; sqlite-vec is not linked — R43.10), local embeddings (`nomic-embed-text`), swappable VectorStore |
| Sync | Incremental connector polls/cursors; folder watcher + hourly scan fallback; hash dedup; delete cascades |
| Data model | Uniform entity graph (entities + typed relationships + provenance); contradictions as external layer |
| Answers | Planner → executor → fusion → graph walk, section-level citations, cheap default BYO model |
| Identity | Hash dedup in-source; cross-source duplicate proposals for manual merge |
| Billing | Personal local free forever; Team self-hosted = one-time platform license (perpetual + 1yr maintenance), offline ed25519 key (`rust/crates/anchorcore/src/license.rs`, issue via `scripts/make_license.py`); hosted pricing TBD from pilots. *(Superseded 2026-10-21: Team pricing is quote-only and the fee structure is undecided — see `rust/docs/decisions.md`.)* |
| Security | OS keychain for credentials (macOS/Windows; encrypted-file fallback elsewhere); secrets never in DB/config |

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
| Summary | Instant Payments MVP excludes incoming payments |
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
| Team self-hosted (now) | One-time platform license, perpetual incl. 1yr support/updates; no per-seat, no numbers on site. *(Superseded 2026-10-21: Team pricing is quote-only and the fee structure is undecided — see `rust/docs/decisions.md`.)* |
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
| 6 — Packaging | Windows + macOS apps (+ Linux zip), Ollama-first | Downloadable zips via release.yml |

---

## 14. Backlog

Priorities: P1 = testers hit it during validation, P2 = quality/trust, P3 = later.

### Shipped (compact log)

DONE items are one-line verdicts — full narratives lived here until 2026-10-19 (see git history). Rust-era engineering detail: `rust/BACKLOG.md` shipped log.

| Item | What shipped |
|---|---|
| B1. Error transparency + logs | `system_events` + System tab + `/system/logs` download; secrets redacted in logs/UI/config. |
| B2. Progress feedback | Background jobs (202 + polling + history + progress bars) + busy states + running-jobs badge. |
| B3. Dispute tracking | `disputes` audit trail + counter; disputed entities excluded from Q&A by default. |
| B4. LLM config in app | Settings tab + SecretStore keys + runtime-mutable `app_settings` + test-connection. |
| B5. Full-document chunking | Full-text chunk + embed; windowed classifier input instead of truncation. |
| B6. Embedding backfill | Startup + 15-min retry loop; pending count in health/status. |
| B7. Source editing | `PUT /sources/{id}` + edit form; per-key secret-file fix. |
| B8. First-run wizard | `/system/onboarding` + `OnboardingWizard` overlay (checks → connect → ask). |
| B9. Document type coverage | `extract.rs`: Office/ODF/RTF/EML/PDF/HTML/text + code languages + `.gitignore`-aware repo walk (R24.2+R24.3). |
| B10. Model config via UI | Superseded by B4. |
| B11. Parallel classification | Bounded concurrency (default 4) + Ollama tuning docs. |
| B12. Hybrid + cleaning | Text cleaning, heading chunking, FTS5 keyword leg; ACME/RFND probes ≥0.89. |
| B12.1. RRF + diversity + recency | RRF fusion, age decay, per-item cap, context expansion, near-dupe dedupe. |
| B13. Dev hardening | Start scripts, config validation, Alembic, CI, background jobs. |
| B15. Projects | `projects` + scopes + `/qa` `project_id` + header picker. |
| B17. Planner→Executor→Synthesis | Deterministic planner + executor + N-way RRF + `who_knows` tool. |
| B18. Distillation | Chat-window Q&A units (`kind=distilled`) + IDF-gated embedding. |
| B19. Newest-first chat | Ask renders the latest turn on top. |
| B20. Launchable package | PyInstaller exe/app (legacy Python era; superseded by the Rust binary). |
| B21. Sample dataset | `sample/` corpus + `sample-dataset.md` guide. |
| B23. Cloud classifier/embed | Classifier + embedder decoupled from Ollama (provider sections, keychain keys). |
| B24. macOS build | `build.sh` + ad-hoc sign (legacy Python era). |
| B25. Release rebuild | `release.yml` builds zips on every `v*` tag. |
| B26. Doc-aware classification | Doc-type pre-pass + `window_text` review context + 16k windows + hash skip. |
| B27. Review context | `GET /entities/{id}/context` + neighbour/highlight/full-doc panels. |
| B28. Drive skeleton | `gdrive` connector + mocked tests (OAuth + local mirror still open). |
| B30. PII gating | Source labels + provider trust + `/qa/public` (auto-detect stack later removed, R35.1). |
| B31. Rust port | v1.0.9+ — Rust is the shipped backend ([eval](./archive/rust-port.md)). |
| B32. Graph retrieval | 1–2 hop relationship walk (kind weights, hop decay, fan-out cap). |
| B33. vec0 index | `vec_chunks` + triggers + Python-scan fallback + latency in status. |
| B34. Backend hardening | Single `__version__`, route dedup, shared secret fields, bounded jobs (2). |
| B35. Pipeline decomposition | `chunking`/`hashing`/`distill` extracted; commit-before-LLM tested. |
| B36. Frontend hardening | `/projects` proxy, abort+timeout, `ErrorBoundary`, `stripPlaceholders`. |
| B37. Frontend decomposition | SourcesTab split, `useJobsPoll`, `ProjectsContext`, theme tokens. |
| B38. Testing uplift | vitest + eslint in CI, paginated duplicates, `isolated_db` fixture. |
| B39. Thin label gate | `sources.label` + `ANCHOR_CLOUD_TRUST`; cloud blocked on sensitive/pii. |
| B40. Stone & Sage | v1.0.11 palette (`theme.ts` + site). |
| B41. Doc-grouped Entities | v1.0.11 grouped view (PII shield later removed, R35.1). |
| B42. Review flood fix | v1.0.11: selective verify, banner persist, needs-review flag. |
| B43. Business-scale corpus | v1.0.11 generator + Rust NOT NULL fixes. |
| B44. REST connector | Generic JSON API source with field mapping + preview. |
| B46. Archive-Python consumers | CI green incl. conformance; license gate sound; tag pinning recorded. |
| B47. Website deploy | anchorcore.dev live (Workers) + CI deploy on tags. |
| B52. Onboarding probe | Real Ollama reachability probe (fixed via B54). |
| B54. Dummy-proof models | Readiness probe + pull endpoints + wizard install + sticky degraded banner. |
| B56. Watcher edits | mtime+size fingerprints; in-place edits auto-sync. |
| B57. Deleted-file purge | Hard-delete of missing files on sync (`rest`/empty guarded). |
| B58. Degraded honesty | `mode`/`retrieval` flags + per-answer banner + strict query embed. |
| B59. Single-container Team | R19.1: one-container Rust image + `license.rs` gate + GHCR publish. |
| B60. Classifier wiring + PDF safety | R22.1/R22.2: real LLM classify path + `documentation` type + PDF extract + skip-never-purge. |
| B62. Entities clarity | R31.1+R31.2 (2026-10-14): option A view + Review fold. |

### Open

### B16. who_knows — expertise queries (P1 — Cerebras learning)

**Status (2026-10-26): CLOSED.** The who-knows leg and graph expansion were deleted in R45.9 (no production callers); ownership is answered by `GET /actions` (`Owner:` record fields, R45.8). Reopening needs a new owner decision — a ranking built from model-extracted authorship would contradict R45.18.

**Problem:** a top question in every org is "who is the expert in Y?" Cerebras surfaces people with demonstrated expertise from the index.

**Scope:**
- Query over entity graph + owners: entities authored by/assigned to a person, weighted by entity confidence + recency + count
- `who_knows(topic)` tool (MCP + UI); people surfaced with their evidence (which entities prove expertise, cited)

**DoD:** ask "who knows about migrations" and get a ranked person list with cited evidence.

### B22. Live integration validation: Jira / Linear (P1)
**Problem:** the Jira and Linear connectors exist but have never hit real instances. "Connects to your tools" is claimed but unproven.

**Scope:**
- Jira: validate against a real sandbox instance (Atlassian free tier + API token) — incremental cursor, comments→doc text, author/assignee mapping, error surfaces; fix whatever breaks
- Linear: validate the shipped connector (API key auth, issues/cycles, incremental by updatedAt) against a real workspace
- Recorded JSON fixtures + mocked-fetch tests so CI validates connector logic without credentials (per B21 sample data)
- Troubleshooting pass in README: how to create a Jira API token / Linear API key, expected permission scope

**DoD:** a Jira source and a Linear source sync real data end-to-end on a sandbox; connector logic covered by fixture tests in CI.

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
(`38` entities); ask #1 `5` citations (instant-payments decision),
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
where. Update `hosted-cloud-plan.md` + `v2v3-scope.md` with the outcome.

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

### B55. Retrieval/answer eval harness (P1)

**Problem:** retrieval quality is where this product lives or dies, and today
it is measured by hand. The good numbers in this doc (B12: §372 at 0.931,
§434 at 0.892) are one-off live probes — unrepeatable, untracked, and blind
to regressions. The retrieval stack keeps gaining knobs (RRF weights, age
halflife, per-item caps, tag threshold, TOC depth) with no way to tune them
except vibes; a change that helps ACME questions can silently sink billing
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

**Status (2026-10-26):** the deterministic half shipped — `cargo test -p anchorcore --test retrieval_eval` (golden pairs, fused recall 1.00 gate, runs in CI; fixture discipline in `tests/eval_data/README.md`). Open: only the model-judged answer eval, which matters for `local` mode alone (in `agent` mode the agent composes the answer) — P3.

### B61. Backup / export / device migration + encrypted device sync (P1 — plan 2026-10-11)

**Problem:** there is no way to move AnchorCore to a new device or restore after data loss. `~/.anchorcore` holds the DB, but secrets live in the OS keychain (or a Fernet fallback whose key file sits next to the ciphertext), folder paths and source refs are machine-specific, and re-syncing cannot restore review work (verifications, disputes, merges, PII decisions). The future hosted tier promises sync between devices ("AI already knows"); without a design it becomes "upload your company memory to our servers".

**Scope:** Phase A (local, shippable): UUIDv7 uids, portable `.anchorcore-backup` export/import (versioned JSONL schema, zip-slip/zip-bomb safe, transactional), optional passphrase-encrypted secrets bundle (Argon2id + XChaCha20-Poly1305), CLI + System-tab UI, docs + acceptance script — `rust/BACKLOG.md` Phase 25 (R25.1–R25.7). Phase B: the same op log synced through a user-chosen cloud folder (no server, no account). Phase C: zero-knowledge hosted/Team relay — account master key from passphrase, per-device envelopes + revocation, signed encrypted op log in R2; the server sees ciphertext + minimal metadata only. Full design, threat model, decisions and acceptance scripts A1–A5: `docs/plan-backup-sync.md`.

**DoD:** from a release build, a Windows + macOS device migration restores answers and review state with only tokens re-entered (A1); exports never contain plaintext secrets (CI grep test); plan §8 decisions signed off before Phase B starts.

**Status (2026-10-12):** Phase A **shipped** — R25.1–R25.7 (uids, export, import, secrets bundle, CLI, System-tab card, docs). Verified on Windows end-to-end (live export from a seeded instance → import into a fresh data dir → hybrid retrieval + re-embed job; replace with safety backup; wrong passphrase leaves nothing behind; secrets restore as `***set***`); the macOS leg of A1–A3 still needs a run on a Mac. Phase B/C remain design-only until the §8 decisions are signed off.

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
| — | B9 Document type coverage | ✅ DONE (R24.2 + R24.3) | Business formats (Office/ODF/RTF/EML/PDF) shared by folder + Drive, plus code languages and `.gitignore`-aware repo walking; legacy `.doc`/`.ppt` = export first |
| — | B16 who_knows (full) | CLOSED 2026-10-26 | the who-knows leg was deleted in R45.9; ownership is `GET /actions` |
| 5 | B29 remainder: CHANGELOG + hosted prep | **P1** | website shipped; changelog discipline + pilot prep open |
| 6 | B28 Drive OAuth polish | **P2** | OAuth browser flow + `data/drive/<name>/` mirror; the token-refresh half is `rust/BACKLOG.md` R43.14 |
| 7 | B45 Slack | **P2** | export-ZIP import local (free); live connector = Teams/hosted feature |
| — | B47 deploy website | ✅ DONE | live at anchorcore.dev (Cloudflare Workers Static Assets) |
| 10 | B48 end-to-end test | **P1** | Windows §1 + MCP §2 PASS (simulated-clean); Mac §3 rides the R41.2 DMG clean-Mac run (with backup A1–A3); Docker §4 awaits owner |
| 11 | B49 hosted design | **P2** | owner design session → decisions in hosted-cloud-plan.md/v2v3 |
| 12 | B50 sample corpus in releases | **P1** | wizard sample path dead on release installs (from B48) |
| 13 | B51 console feedback for exe | **P2** | partial: `attach_parent_console` (`cli.rs`) covers the backup/import subcommands; a plain server run from a terminal is still silent |
| — | B52 onboarding Ollama probe | ✅ DONE (via B54) | real probe + install guidance in wizard |
| — | B54 dummy-proof model setup | ✅ DONE (CI gates Rust) | guided wizard + Pull button + sticky degraded banner |
| 15 | B53 citation path cosmetics | **P3** | verbatim `\\?\` refs + empty `path` on Windows (from B48) |
| 16 | B55 answer-eval half | **P3** | retrieval gate shipped (`tests/retrieval_eval.rs`); left: model-judged answer eval, `local` mode only |
| — | B56 watcher content edits | ✅ DONE (cargo 118/118) | mtime+size fingerprints; edits auto-sync; Obsidian promise holds |
| — | B57 deleted-file cleanup | ✅ DONE (cargo 118/118) | hard-delete on sync; `rest`/empty guarded; job result gains `purged` |
| — | B58 degraded-answer honesty | ✅ DONE (cargo 118/118, vitest 22/22) | `mode`+`retrieval` flags; per-answer banner; strict query embed |
| 20 | B59 single-container Team image | ✅ DONE (R19.1) | site promises "One Docker container" but `hosting/` is a two-service Python stack; ship the Rust single container (or reword) |
| 21 | B60 classification wiring + PDF purge safety | ✅ DONE (live) | Rust never called the LLM classifier → keyword rules fabricated decisions from process docs; PDF stub + B57 purge then deleted the ingested PDFs |
| 22 | B61 backup / export / device migration | **P1** | no way off the device; secrets make "copy the folder" unsafe; zero-knowledge sync is the hosted differentiator — **Phase A shipped 2026-10-12 (R25.1–R25.7); Phases B/C gated on the §8 sign-off** |
| — | B62 Entities clarity | ✅ DONE (R31.1+R31.2) | option A view + Review fold — single attention queue, honest states (2026-10-14) |

*Companion docs: [architecture.md](./architecture.md), [packaging.md](./packaging.md), [mcp.md](./mcp.md), [archive/rust-port.md](./archive/rust-port.md), [design-system.md](./design-system.md)*
