# AnchorCore — Full Code & Feature Review 2026-08-21

**Reviewer:** senior tech lead / engineering director (external)
**Scope:** whole product — shipped Rust binary (`rust/crates/anchorcore`, `v1.0.9`), legacy Python backend (`backend/`, conformance only), React SPA (`frontend/`), hosting (`hosting/`), CI (`ci.yml` / `release.yml`), docs.
**Commit reviewed:** `0551f8c` (working tree clean) — Rust retired Python at `7131cfe`.
**Method:** full source read of the Rust answer/retrieval/pipeline/jobs/auth/secrets/settings/system/sources/entities/frontend modules; built and ran the Rust test-suite; ran the Python conformance suite against a live Rust binary; ran targeted black-box probes (non-Latin retrieval, job cancel). All claims below are **reproduced**, not inferred.
**Backlog:** the remediation plan lives in `rust/BACKLOG.md` **Phase 7–10** (`R7.1`–`R10.9`). This doc is the justification; the backlog is the work order.
**Supersedes:** `docs/port-review-2026-08-21.md` (optimistic "ready for cutover") and corrects `docs/port-review-2026-08-21-lead.md` (whose P0s were **not** all landed before Python was retired — see §6).

---

## 0) Executive verdict

**Architecture is sound, the release was premature, and two security holes are release-blocking.**

- The contract-first port, `spawn_blocking`-everywhere SQLite discipline, secret masking, and the agent-guide culture are genuinely good and should be preserved.
- But the **shipped artifact does not implement the product it is sold as**: the "hybrid RAG" answer path is **keyword-only** (vector search is written but never called), ranking is corrupted by a constant-score bug, and multiple advertised features are stubs or no-ops.
- The **"124/3 vs Rust" conformance claim is not reproducible** on a clean checkout. I measured **115 passed / 8–9 failed / 3 skipped**, and the documented reproduction command contains env-var typos (`ANCHR_` vs `ANCHOR_`) that make it impossible to run as written.
- Python was retired (`7131cfe`) against the explicit instruction in `docs/port-review-2026-08-21-lead.md:8`: *"Do not retire Python until the P0s below land."* Several of those P0s are still open today.
- **Do not tag another `v*` until R7.1–R7.2 (security), R8.1–R8.2 (retrieval), R9.1–R9.2 (conformance/CI) land.**

---

## 1) Verification log (what I actually ran)

| Check | Command | Result |
|---|---|---|
| Build | `cargo check -p anchorcore` | **0 warnings** (lead review's 71 warnings are resolved) |
| Rust unit tests | `cargo test -p anchorcore` | **44 passed / 0 failed** (5.3s) |
| Conformance (fresh, matched env) | Rust on `:8123` + `ANCHOR_TEST_RUST_URL pytest backend/tests -q --ignore=test_mcp.py` | **115 passed / 9 failed / 3 skipped** (run 1) |
| Conformance (same server, second pass) | same | **115 passed / 8 failed / 3 skipped** (run 2; distillation + test-connection flaked in run 1) |
| Documented repro | `docs/port-review-2026-08-21.md:80-83` (uses `ANCHR_*` env) | **cannot run as written** — `ANCHR_*` is ignored by Rust (`db.rs:8`, `settings.rs:43`); server falls back to `data/anchorcore.db` while pytest uses a temp DB → env mismatch (I measured **43 failures** in this broken mode) |
| Non-Latin retrieval probe | Ingest 9KB Cyrillic doc, ask in Cyrillic | returned **stale English test chunks** (score `0.016`); the Cyrillic doc is unfindable |
| Job cancel probe | `POST /sources/:id/reclassify` then `POST /sources/jobs/:id/cancel` | cancel returns `cancelled:true`, but the job **ends `failed` with error "job cancelled"** and the source is marked `error_count+1` |
| B30 trust flag | `test_cloud_answer_trust_flag_allows_sensitive` | fails (citations empty) — see §5.2 |
| Version sync | `test_version_is_single_source_of_truth` | fails in **pure Python mode too** (test hardcodes `1.0.8`) |

---

## 2) Security — S0 (release-blocking)

### S0-1. Local mode: any website can read and destroy the user's entire memory
`main.rs:169` `.layer(CorsLayer::permissive())`; app binds `127.0.0.1:8000` (`main.rs:173`); with auth disabled (default) **every** endpoint is unauthenticated.
A malicious page can `fetch('http://127.0.0.1:8000/entities')`, `.../pii/review`, `.../system/logs`, `.../sources` and read the responses (permissive CORS), then `DELETE /sources/{id}`, `POST /sources/{id}/reclassify`, merge entities, and rewrite model settings. This is your "private company memory" being **exfiltratable and destructible by any webpage the user visits** — no DNS rebinding needed. There is no `Origin`/`Host` validation anywhere.

*Fix:* validate `Host`/`Origin` against an allowlist; replace `CorsLayer::permissive()` with a narrow allowlist; require a per-session CSRF token (custom header) for all mutating requests when auth is off. Backlog: **R7.1**.

### S0-2. Hosted auth protects writes, not reads
`auth.rs:150-158` exempts all `GET/HEAD/OPTIONS` from auth. In hosted mode (the entire point of B40), these are **public**:
`GET /sources`, `GET /entities` (returns `window_text` = full classifier input), `GET /pii/config`, `GET /pii/review`, `GET /system/logs/{name}` (downloads full logs), `GET /settings`.
The PII/sensitive gating (B30/B39) only applies to the LLM answer path — the **API itself leaks the data**. Combined with S0-1's CORS this is remotely exploitable.

*Fix:* route-based authorization, not method-based. Every endpoint that returns content/config/health details must require a token when `auth_enabled()`. Backlog: **R7.2**.

---

## 3) Retrieval is not RAG — P0 (the core value proposition is missing)

1. **Vector search is dead code in the shipped path.** `vector_search` is implemented (`retrieval.rs:160`) but **never called**. `answer.rs:174` `retrieve_sync` and `answer.rs:301` `search_sync` always pass `vector_hits=None`; the query is never embedded. Grep confirms the only reference is the definition itself.
2. **Ranking is corrupted.** `fuse_and_rank` (`retrieval.rs:627-631`): when only one list exists (always, today), every hit is assigned a **constant `1/(RRF_K+1)=1/61`**, discarding the BM25 scores the FTS query just computed. Ranking then reduces to age-decay + insertion order. I confirmed empirically: all returned citations scored exactly `0.016`.
3. **Embeddings are computed and stored, then never read** (`pipeline.rs:434-456` embeds and calls `sync_vec`; no reader). This is wasted ingestion cost on every sync.
4. **Telemetry is fabricated.** `system.rs:83-88` hardcodes `"backend": "vec0"`, `"calls": 0`, `"vec0_calls": 0`, `"avg_latency_ms": 0.0` — the System tab reports a vector engine that is never used.
5. **Non-Latin content is unfindable.** `fts_match_query` tokenizes with `[a-z0-9_§\-]+` (`retrieval.rs:35`) — Cyrillic/CJK/accents are dropped; the stopword list (`retrieval.rs:36-44`) is English-only. My Cyrillic probe returned stale English leftovers for a Cyrillic question. For any international team this is an empty tool.

Backlog: **R8.1** (wire vector + fix RRF/rank preservation), **R8.2** (non-Latin tokenization).

---

## 4) Feature gaps vs. what README/UI/AGENTS claim — P1

| # | Claimed | Shipped reality | Location |
|---|---|---|---|
| 4.1 | "Hybrid RAG (vector + keyword), RRF k=60, age decay, diversity cap" | Keyword-only; scores constant 1/61; BM25 ranks thrown away | `answer.rs:174,301`; `retrieval.rs:627` |
| 4.2 | "Follow-ups rewritten into standalone queries using conversation history" | `generate_answer` ignores history (`_history`); no rewrite anywhere; UI copy ("follow-ups work") is wrong | `answer.rs:335`; `AskTab.tsx:129` |
| 4.3 | Jira / GDrive connectors | `run_sync_inner` returns `vec![]` for both — **silent no-op**, job still reports `done` | `pipeline.rs:113-119` |
| 4.4 | Auto-sync scheduler (`jira_poll_minutes`/`folder_scan_minutes`) | `Scheduler::reload_sources` is a stub that aborts handles and does nothing; `spawn_for_source` ticks and only logs | `scheduler.rs:30-44,58-65` |
| 4.5 | Job concurrency bounded at 2 | Labels are bounded but **execution is unbounded**: `sources.rs:302` spawns a pipeline per sync regardless of queue; `pipeline.rs:64` then flips `pending`→`running`, so `MAX_CONCURRENT=2` is cosmetic | `sources.rs:302`; `jobs.rs:98`; `pipeline.rs:64` |
| 4.6 | B39 PII gate during classification/distillation | `pipeline.rs:219` computes `gated` then `let _gated = …` (unused). Cloud LLM classify/distill only runs for doc-type detection; extraction is always rules | `pipeline.rs:219,280,383` |
| 4.7 | "124/3 vs Rust green", "CI `cargo check 0`" | Not reproducible: **115/8/3**; CI runs Python tests only, never the Rust binary | §5 |
| 4.8 | Hosted = "same app against Postgres" | `hosting/Dockerfile` builds the **Python** FastAPI backend; the shipped Rust binary has no Postgres path (`db.rs:19` comment only) — two divergent codebases | `hosting/Dockerfile:1` |

---

## 5) Conformance-claim autopsy (diligence detail)

### 5.1 The documented reproduction command is broken as written
`docs/port-review-2026-08-21.md:80-83` (and `AGENTS.md` gotchas) use `ANCHR_DATA_DIR`, `ANCHR_DATABASE_URL`, `ANCHR_SECRETS_NO_KEYRING`, `ANCHR_CLASSIFIER_TIMEOUT`. The Rust binary reads **`ANCHOR_`** (`db.rs:8`, `settings.rs:43`, `secrets.rs:15`). Running the command as written starts the server on `data/anchorcore.db` while pytest uses its own temp DB → the server and the harness talk to **different databases** (I measured **43 failures** in this mode). The command must be fixed or it will keep producing false negatives.

### 5.2 The B30 "trust" test passed via a removed name-hack, not by behavior
`R6.4`'s backlog note admits the fix was a *"`'%trusted%'` hack"* matching a **source name** (`trusted-sens`) in Rust `answer.rs`. That hack is gone (current `is_answer_trusted` at `answer.rs:263` reads `answer_base_url` + `ANCHOR_CLOUD_TRUST`). The test `test_cloud_answer_trust_flag_allows_sensitive` then **fails** because it monkeypatches Python's `settings_svc` / `os.environ` in the test process — invisible to the separate Rust server. **The historical "124/3" number was achieved with a hack that has since been removed.**

### 5.3 Conformance tests reach into Python internals via HTTP mode
By construction these cannot pass against a Rust server:
- `test_settings_secret_masked_and_stored_in_keystore` — asserts on `settings_svc.secrets.get(...)` (Python in-process store).
- `test_secret_fields_stay_keychain_backed` — reads `app.secrets.SecretStore(settings.data_dir/...)` directly.
- `test_cloud_answer_trust_flag_allows_sensitive` / `test_sensitive_source_blocked…` — monkeypatch Python's `settings_svc`.

### 5.4 Genuine failures that ARE product signals
- `test_smoke.py::test_cancel_running_job` — **real pipeline bug**: cancel leaves the job `failed` with `"job cancelled"` and bumps the source's `error_count` (`pipeline.rs:136-138` → `sync_source` marks `failed` + `record_sync_error`). Sources tab shows an error state for a deliberate user cancel. Backlog: **R9.1**.
- `test_system.py::test_version_is_single_source_of_truth` — **stale test**: hardcodes `"1.0.8"` (`test_system.py:48`); fails in pure Python too. Nothing in CI bumps it. Backlog: **R9.2**.
- `test_projects.py::test_project_crud` — shared-DB accumulation (`/projects` not empty), violating the repo's own "assert deltas, never emptiness" gotcha.
- `test_smoke.py::test_folder_watcher_picks_up_new_files` — the known flaky watcher test, still flaky.

### 5.5 CI never tests the shipped artifact
`ci.yml` runs Python pytest and the frontend build. **No job builds the Rust binary and runs the conformance suite against it.** `release.yml` builds the binaries but runs zero tests. The "124/3 green" is a manual, undocumented-env ritual. Backlog: **R9.2**.

---

## 6) Why "124/3 green ⇒ shipped" was the wrong call

`docs/port-review-2026-08-21-lead.md` (same day) already flagged P0s:
- "124 green is half-Python — Rust retrieval hardening not exercised" (§3.1) → **still true**: `vector_search` unused, ranking broken.
- "Embeddings never written — vec0 path is dead code" (§3.2) → **partially fixed** (now written) but **never read** — a new flavor of the same defect.
- "Auth middleware missing" (§7) → **partially fixed** with a method-based exemption that leaves all reads public (S0-2).
- "main.rs watcher monolith" (§3.3) → fixed (extracted to `watcher/service.rs`).

Python was retired anyway. Several P0s live on in new clothes. The release decision did not respect the review's cutover gate.

---

## 7) What's genuinely good (preserve, don't regress)

- Contract-first, file-disjoint, agent-sized work slicing and the living `AGENTS.md` culture.
- `PRAGMA foreign_keys=ON` kept everywhere; `spawn_blocking` discipline for SQLite; per-key `secrets.enc.<sha256>` fallback with `***set***` semantics; `RedactingFormatter` intent.
- Secret masking on `GET /sources/{id}/config`; `.env` correctly gitignored (note: `backend/.env` holds what looks like a real `sk-…` key locally — rotate if that file ever touched any repo/history).
- `cargo check` clean, `cargo test` 44 green, SQL parameterization fixes (P1 4.5 from lead review) landed.
- Frontend quality is high: error boundaries, abort handling, typed API client, token-based theming, TanStack Query, public-only toggle.
- Migration strategy (10 idempotent `rusqlite_migration` steps + Python-DB fast path) is pragmatic and safe.

---

## 8) Prioritized remediation (→ `rust/BACKLOG.md`)

| Prio | Task | Where |
|---|---|---|
| S0 | Origin/Host allowlist + narrow CORS + local-mode CSRF token | **R7.1** |
| S0 | Route-based authz (all content/config GETs protected when auth enabled) | **R7.2** |
| P0 | Wire vector search into the answer path + fix single-list score flattening (preserve BM25 ranks) | **R8.1** |
| P0 | Non-Latin tokenization (Cyrillic/CJK/accents) | **R8.2** |
| P1 | Job cancel → `cancelled`, not `failed`; no `record_sync_error` on cancel | **R9.1** |
| P1 | Conformance honesty: split HTTP-only vs Python-internal tests, fix stale version test, **CI job that tests the shipped Rust binary**, fix documented command env typos | **R9.2** |
| P1 | Follow-up rewrite + history into generation | **R10.1** |
| P1 | Jira/GDrive ingestion or fail loudly (no silent `done`) | **R10.2** |
| P1 | Real scheduler auto-sync (or drop the claim) | **R10.3** |
| P1 | Enforce `MAX_CONCURRENT` at execution time, not labels | **R10.4** |
| P2 | `/qa` event spam (2 warnings per question) → only on real degradation | **R10.5** |
| P2 | UTF-8-safe truncation (`answer.rs:151,340,348,369,371`, `bin/mcp.rs:58`) | **R10.6** |
| P2 | Hosting parity decision (Rust in `hosting/` or document Python-only) | **R10.7** |
| P2 | Login rate-limit/lockout; bound `/system/logs`; stop leaking `data_dir` in `/health` + `/system/status` | **R10.8** |
| P2 | Dead code + drift: `ask_stub`, `vector_search` duplication, `scripts/sync_version.py` check in CI, doc cleanup ("124/3") | **R10.9** |

---

## 9) Appendix — reproduce

```bash
# Rust tests
cd rust && cargo check -p anchorcore && cargo test -p anchorcore

# Conformance (CORRECT env — both sides share the same DB + data dir)
rm -rf /tmp/ac-conf && mkdir -p /tmp/ac-conf
cd rust && ANCHOR_DATA_DIR=/tmp/ac-conf ANCHOR_DATABASE_URL=sqlite:////tmp/ac-conf/test.db \
  ANCHOR_SECRETS_NO_KEYRING=1 ./target/debug/anchorcore --port 8123 &
cd .. && ANCHOR_DATA_DIR=/tmp/ac-conf ANCHOR_DATABASE_URL=sqlite:////tmp/ac-conf/test.db \
  ANCHOR_SECRETS_NO_KEYRING=1 PYTHONPATH=backend ANCHOR_TEST_RUST_URL=http://127.0.0.1:8123 \
  backend/.venv/bin/python -m pytest backend/tests -q --ignore=backend/tests/test_mcp.py
# → 115 passed / 8 failed / 3 skipped (NOT 124/3)

# Cancel bug
curl -s -X POST localhost:8123/sources -d '{"connector":"folder","name":"c","config":{"path":"/tmp"}}'
curl -s -X POST localhost:8123/sources/1/reclassify
curl -s -X POST localhost:8123/sources/jobs/1/cancel   # then GET job → status "failed"
```
