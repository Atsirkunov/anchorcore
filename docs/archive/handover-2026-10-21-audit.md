# Handover 2026-10-21 — full-repo audit (Rust tree, v1.0.31)

For the next agent / owner. Read-only audit of `main` at `4bb7443` (tag
`v1.0.31`, in sync with `origin/main`). No code was changed. Findings were
verified by reading the code paths end to end; severities and confidence
are stated per item. Python `backend/` is out of scope.

## Calibration corrections (owner review, 2026-10-21 — applied during the backlog fold)

- **M4 split:** Drive OAuth refresh is **High** (M4a — a raw access token expires in ~1 h, so Drive sync breaks by design; filed as R43.14); cursors/streaming/size caps stay **Medium** (M4b, R43.13).
- **M7 mechanism:** the `.zip` truncates either way — a middle-drop misaligns later vectors, a tail-drop silently leaves `None`. Test both (R43.6).
- **H2 gate:** `clean_doc_text` mirrors the archived Python pipeline — check the conformance suite for cleaning-pinned expectations before changing (R43.2).
- **H1 sequencing:** the sqlite-vec link is the risky leg (rusqlite bundled build + C amalgamation must compile on Windows MinGW-GNU and all 3 CI OSs) — land the interim slim scan first (R43.9), link second (R43.10).
- **H8 scale:** High as a scale gate; Medium impact on small local corpora. Already filed as R30.3 (same fix); H4 likewise duplicates R30.1.
- Base `4bb7443` is one commit behind `a0e6d4e` (Linear fix); no finding is affected. The staged-`backend/` heads-up below self-resolved (tree clean).

## State of the tree

- `cargo test -p anchorcore` green (exit 0) on macOS with
  `ANCHOR_SECRETS_NO_KEYRING=1 ANCHOR_OPEN_BROWSER=0`.
- **Heads-up:** 89 `backend/` files are *staged* (leftover from a
  conformance restore). `git add -A` + commit would resurrect the archived
  backend on `main`. Clear with `git restore --staged backend && rm -rf backend`
  unless intentional.
- Not run: conformance suite, lizard gate, release build, `cargo audit` /
  `cargo outdated` (not installed on this box).

## Map

One Axum process (`main.rs`). Layers outer→inner: Host/Origin →
CORS → CSRF (auth off only) → JWT auth (`ANCHOR_AUTH_SECRET` set) → handler.
SQLite WAL, no pool — every request/task opens a connection
(`init_db`/`open_db`).

- **Ingest:** job row → detached `tokio::spawn` → `jobs.rs` (2 slots, DB
  status) → connector fetch → `extract.rs` → `upsert_doc` →
  `store_doc_chunks` (clean → chunk → delete+insert in `BEGIN IMMEDIATE`)
  → LLM classify per 16 KB window → summaries/tags → embed.
- **Ask:** follow-up rewrite → query embed → `orchestrate_fusion`
  (TOC/flat legs → RRF → slots → filename boost → `parent_return`) →
  label gate → LLM answer → citations.
- **External:** Ollama / OpenAI-compatible (classify, embed, answer), Jira,
  Linear, Drive, generic REST, OS keychain.

Risk concentrates in: auth middleware + missing authz (Team), source-scope
filtering in retrieval (the `/qa/public` boundary), chunking/cleaning
(silent data loss), job scheduling/concurrency, blocking work on the async
runtime, outbound fetches to user-supplied URLs.

## Executive summary

Retrieval ranking, migrations, and backup are carefully engineered and well
documented. The edges are weak: **Team edition is not deployable as-is**
(open or unusable), **retrieval won't scale to "many GB"** (the vector
index the docs describe is not compiled in — every query is a full scan),
and **chunking silently deletes/fragments content** in PDF/DOCX/HTML docs.

Top 3 risks:
1. Team auth: unauthenticated content reads + open signup + no authz.
2. `/qa/public` and project scoping leak via the filename boost.
3. Brute-force N+1 vector scan + per-second watcher directory walk.

## Findings by track (staging view)

The product is local-first today: the goal now is the crispest possible
retrieval, with access/security for the hosted Team image as a later
stage. The severity labels below (C/H/M/L) are absolute risk; this table
re-sorts the same findings into **tracks** and the **stage** they belong to.

| Track | What it covers | Findings | Stage |
|---|---|---|---|
| **A. Content fidelity** | What ends up in chunks — nothing lost, mangled, or mis-split | H2 cleaning deletes content · H3 heading over-fire / no small-chunk merge · M2 code/indent mangling · M1 Jira ADF dropped · M9 line ranges only for markdown | **1 — now** |
| **B. Retrieval & answer quality** | Ranking, scoping, embeddings, answer composition | C3 filename boost ignores scope · M7 embed alignment + nomic prefixes · H6 answer timeout / `num_ctx` truncation · L classify-window overlap | **1 — now** |
| **C. Local reliability & scale** | Sync/jobs behaving, runtime not starved, works at "many GB" | H1 full-scan vector search · H4 no job dedupe · H5 panic leaks job slot · H8 watcher per-second walk · M3 lock held while chunking / full re-embed · M4a Drive token refresh (High) · M4b cursors/caps/streaming · M8 import 2 MB cap · patterns 1, 2, 4, 5, 6 | **1–2 — alongside A/B** |
| **D. Access management** | Who can log in, who can do what, UI auth | C1 unauthenticated GETs · C2 open signup / no roles · H7 SPA has no login + compose Ollama untrusted · M6 login rate limiting · M5 keychain not namespaced per data dir | **3 — before hosted** |
| **E. Security hardening** | Outbound/inbound attack surface for a networked deploy | H9 SSRF via REST preview / test-connection · L personal edition on 0.0.0.0 ungated · L JQL / Drive `q` unescaped · pattern 7 fail-open design · dependency audit (`cargo audit`) | **3 — before hosted** |

Cross-track notes:
- **C3 is staged with retrieval, not security.** It lives in fusion code,
  is a one-function fix, and breaks *project scoping* locally today (wrong
  answers, not just a leak). It also protects `/qa/public` + MCP
  `public_only`, which local users already rely on.
- **H1 is the scale gate.** Small local corpora won't feel it; anything near
  the "many GB" target will. Do it before the hosted stage, because team
  corpora are where it bites.
- **H6 belongs to retrieval quality** even though it is a timeout: with local
  Ollama it decides whether users see a composed answer at all, and the
  `num_ctx` truncation silently drops retrieved evidence.
- **M5 has a local symptom too** (dev instance vs `~/.anchorcore` sharing
  keychain entries) but only becomes a security issue when hosted.

## Findings

### Critical

**C1 — Item content readable without auth when auth is on**
`auth.rs` `is_public_path`
- Any GET whose path doesn't start with one of 9 hard-coded prefixes is
  treated as a public SPA route. `/items`, `/items/:id`, `/sections`,
  `/sections/:id/chunks`, `/nodes/:id`, `/tags/:id/chunks` are missing.
- Scenario: Team deploy, `curl -H 'Host: localhost' http://team:8000/items/1`
  returns full document content with no token.
- Fix: invert to default-deny (allow `/`, `/index.html`, `/assets/*`, known
  SPA routes, auth endpoints). Add a test that walks every route in
  `build_router` with auth on and expects 401.
- Confidence: high.

**C2 — Open signup, no authorization model**
`auth.rs` `signup_handler`
- Anyone reaching the server can sign up; every user is effectively admin.
- Scenario: LAN user signs up → `PUT /settings` `answer_base_url` to their
  server (stored API key is kept and sent as `Bearer` on the next ask) /
  `POST /backup/export` the whole corpus / create a folder source on any
  container path.
- Fix: close signup after the first user (or invite-only); admin role for
  settings, sources, backup, `/system/*`. Until then document Team as
  single-tenant, trusted LAN only.
- Confidence: high.

**C3 — Filename boost ignores source scope**
`retrieval.rs` `filename_boost_filtered` (called from `boost_and_assemble`)
- Calls `load_hits_filtered(conn, &fresh, None, None, …)`; nothing
  re-filters by scope afterwards.
- Scenario: `POST /qa/public {"question":"what's in salaries.xlsx"}` (or MCP
  `public_only`) surfaces chunks from internal sources — and sensitive ones
  when the provider is trusted (local). Project scoping leaks the same way.
- Fix: pass `scope` into the boost (filter `named_item_ids` by
  `source_id`); add a final scope assert in `orchestrate_fusion`.
- Confidence: high.

### High

**H1 — Vector search is always a full scan with N+1**
`db.rs` `run_migrations` ("This build has no vec0 module linked");
`retrieval.rs` `vector_search_scan_filtered`
- `sqlite-vec` is not a dependency, so `vec_chunks` is a plain table and
  every query falls back to the scan. The scan loads every embedded
  document chunk *with content*, then runs one `SELECT embedding` per chunk;
  source filtering happens in Rust. Runs on the flat path and the TOC floor.
- Scenario: 1M chunks → ~3 GB of embeddings plus all content per ask.
  AGENTS.md "Retrieval is index-backed" is false for the shipped binary.
- Fix: statically link `sqlite-vec` (crate + `sqlite3_auto_extension`).
  Interim: scan `(id, embedding)` only with the source filter in SQL,
  hydrate top-k after. Correct the docs. **Calibration:** linking is the risky leg (rusqlite bundled build + the C amalgamation must compile on the Windows MinGW toolchain and all 3 CI OSs) — land the interim scan first (R43.9), link second (R43.10).
- Confidence: high.

**H2 — Prose cleaning deletes real content**
`chunking.rs` `repeated_lines`, `is_page_number_line`
- Any line ≤120 chars repeated ≥ max(3, 1% of lines) is removed from the
  whole document ("Yes", "N/A", "Approved", "Description", "Total"). Any
  line that is just a 1–4 digit number is dropped (years, amounts, qty).
- Scenario: PDF tables (pdf-extract emits one cell per line) lose values
  and labels from chunks/FTS/embeddings. Raw item text keeps them, so the
  loss is invisible.
- Fix: strip repeats only at page boundaries (first/last N lines per page —
  page markers exist since R36.10); drop bare numbers only next to page
  breaks. Golden tests for table PDFs. **Calibration:** `clean_doc_text` mirrors the archived Python pipeline — check the conformance suite for cleaning-pinned expectations before changing (record the call; R43.2).
- Confidence: high on behavior, medium on corpus impact.

**H3 — Heading heuristic over-fires; prose path has no small-chunk merge**
`chunking.rs` `classify_heading`, `materialize_opts`
- `\d{1,4}(\.\d{1,4}){1,3}` matches `3.5 million users…`,
  `1.0.31 release`, `21.10.2026 notes`, `10.0.0.1 gateway`. Any ALL-CAPS
  line of 4–80 chars (`NOTE`, `ACME CORP`) becomes a level-1 heading and
  pops the numbered hierarchy. Unlike `markdown.rs` (`merge_short`,
  `MD_MIN`), the prose path has no minimum chunk size.
- Scenario: numeric tables and release notes split into dozens of 1–2 line
  sections; tiny chunks win BM25 length normalization (`CANDIDATE_MULT`
  exists to paper over this).
- Fix: numbered headings must be followed by non-numeric text and fit the
  surrounding sequence; ALL-CAPS only when followed by a blank/short line
  and never level 1 inside a numbered tree; port `merge_short` to
  `materialize_opts`.
- Confidence: medium-high.

**H4 — No per-source job dedupe**
`scheduler.rs` `spawn_for_source`, `watcher/service.rs` `trigger_sync`,
`sources.rs` `sync_inner`, `jobs.rs` `create_job`
- Every tick, watcher change, and button click inserts a new job.
- Scenario: a Jira sync longer than 15 min stacks a job per tick; both
  slots can run the same source concurrently (racing `upsert_doc` /
  `store_doc_chunks`) and starve every other source.
- Fix: `create_job` returns the existing pending/running job for
  `(source_id, kind)` — partial unique index or a check inside the
  `BEGIN IMMEDIATE`.
- Confidence: high.

**H5 — A panic inside a job leaks its slot permanently**
`extract.rs` (`pdf_extract::extract_text_from_mem_by_pages`), detached
`tokio::spawn` sites
- pdf-extract/lopdf are known to panic on malformed PDFs; there is no
  `catch_unwind`. The task dies before `finish_job`, so the row stays
  `running` until restart. `PROMOTE_LOCK.lock().unwrap()` panics forever
  once poisoned.
- Scenario: two bad PDFs → job queue dead until restart; next sync hits
  the same files again.
- Fix: `catch_unwind` around extraction → report as `skipped`; wrap job
  bodies so `finish_job` always runs (guard or JoinHandle supervisor);
  recover from mutex poisoning.
- Confidence: high that the guard is missing, medium on frequency.

**H6 — Answer generation effectively broken on local Ollama**
`answer.rs` `generate_answer`
- Hard-coded 10 s timeout. The prompt is up to 8 × 2000 chars (~4–5k
  tokens); on a laptop, prompt eval + generation regularly exceeds 10 s →
  "Answer model unreachable". No `num_ctx` is sent, so Ollama's default
  context silently truncates. The 16 KB classify windows share the
  truncation problem.
- Fix: configurable timeout (default ≥120 s), streaming, pass
  `options.num_ctx` (or use `/api/chat`), budget context in tokens.
- Confidence: high on timeout, medium on truncation (Ollama-version
  dependent).

**H7 — Team edition doesn't work end to end**
- Team refuses to boot on 0.0.0.0 without auth, but the SPA has no login
  and never sends `Authorization` (no Bearer/login code in
  `frontend/src`). With auth on, every API call except the C1 holes is 401.
- `hosting/docker-compose.yml` points at `http://host.docker.internal:11434`,
  which `common::provider_is_local` treats as cloud → sensitive/pii
  sources are never classified/embedded and their answers are refused. The
  only escape is `ANCHOR_CLOUD_TRUST=1`, which also trusts real clouds.
- Fix: SPA login + token handling; add the compose Ollama URL to
  `ANCHOR_TRUSTED_PROVIDERS` in `.env.example` (or opt-in "private hosts
  are local").
- Confidence: high.

**H8 — Watcher walks every folder source about once per second**
`watcher/service.rs` `run`
- The "missed notify" fallback calls `fetch_and_compare` on every loop
  iteration (sleep 1 s): a full `.gitignore`-aware walk + stat of every
  file, synchronously on a Tokio worker.
- Scenario: a 100k-file source pins a core with constant disk I/O and
  starves HTTP workers.
- Fix: run the fallback every 5–15 min, in `spawn_blocking`; dedupe per H4. **Calibration:** High as a scale gate; Medium impact on small local corpora. (Already filed as R30.3 — same fix, no separate item.)
- Confidence: high.

**H9 — SSRF via REST preview**
`sources.rs` `rest_preview_handler` (also `settings` test-connection)
- The server fetches any URL and returns content, or the error body in the
  502.
- Scenario: cloud Team deploy, `POST /sources/rest/preview
  {"base_url":"http://169.254.169.254","list_path":"/latest/meta-data/iam/…"}`
  returns instance credentials. With C2, anyone on the network can.
- Fix: block link-local/loopback/private ranges unless admin-allowlisted
  (resolve then check the IP); cap preview to one page.
- Confidence: high.

### Medium

- **M1 — Jira descriptions and comments dropped.**
  `connectors/jira.rs` `parse_issue`: API v3 returns them as ADF JSON
  objects; `.as_str()` yields nothing, so only summary/status/assignee are
  indexed. Fix: flatten ADF `content[].text` or `expand=renderedFields`.
  Confidence: high.
- **M2 — Splitters mangle code/indentation.**
  `chunking.rs` `split_section_opts` `p.trim()` strips the first line's
  indent per paragraph (Python/YAML symbols >1600 chars).
  `markdown.rs` `split_code`/`split_list`/`split_table` use `split_words`,
  collapsing newlines and indentation in oversized groups. `split_code`
  assumes a closing fence, so an unclosed fence >3000 chars loses its last
  line. Fix: line-based splits, whitespace verbatim. Confidence: high.
- **M3 — Write lock held while chunking; any change re-embeds everything.**
  `pipeline.rs` `store_doc_chunks`: clean+chunk runs inside
  `BEGIN IMMEDIATE`, so a large PDF blocks all writers (others fail after
  `busy_timeout` 5 s). Delete+reinsert means a one-char edit re-embeds
  every chunk. Fix: chunk before `BEGIN`; reuse embeddings by content hash.
  Confidence: high.
- **M4 — API connectors full-fetch into memory every sync.** `fetch_docs`
  always passes cursor `""`. Drive re-downloads every file, no size cap, no
  OAuth refresh (raw access token expires ~1 h). Folder reads whole files
  via `fs::read`, no cap. Fix: persisted cursors, streaming, size caps,
  refresh tokens. **Calibration: SPLIT — M4a Drive OAuth refresh is High (a raw access token expires ~1 h, so Drive sync breaks by design; verify live, then implement the refresh-token flow — R43.14); M4b (cursors/streaming/size caps) stays Medium (R43.13).** Confidence: high.
- **M5 — Keychain secrets not namespaced per data dir.** `secrets.rs`:
  entries are `AnchorCore` / `source:{id}:token` for every data dir — dev
  instances, second installs, or restores that reuse ids read another
  instance's credentials. Fernet key created racily on first use
  (concurrent `SecretStore::new` can write two different keys); write
  errors ignored. Confidence: medium-high.
- **M6 — Login rate limiting.** `auth.rs`: keyed by email only (no peer
  IP), so 5 bad attempts lock out any known user for 60 s, repeatably;
  spraying across emails is unlimited; attempts map grows unbounded.
  Confidence: high.
- **M7 — Embedding alignment and quality.** `embedder.rs`: no check that
  returned vector count equals input count — a dropped item shifts later
  vectors onto the wrong chunks (middle-drop; a tail-drop silently leaves `None` instead — the `.zip` truncates either way, so test both). nomic-embed-text expects
  `search_query:`/`search_document:` prefixes, not sent (known recall loss).
  Confidence: high on code, medium on quality impact.
- **M8 — Backup import likely capped at 2 MB.** `backup.rs`
  `import_handler`: axum 0.7 `Multipart` applies `DefaultBodyLimit` (2 MB)
  and no override is set. Verify with a 5 MB archive. Confidence: medium.
- **M9 — Line ranges only for markdown.** Prose/code/config chunks store
  `line_start/line_end = 0`, so code citations can't point at lines.
  Confidence: high.

### Low

- `classify_windows` 50% overlap sends every char to the LLM twice.
- `sort_hits` `partial_cmp().unwrap()` panics on a NaN score.
- JQL and Drive `q` interpolate config values unescaped.
- `tracing::info!(line, sid)` in the watcher logs a literal `{}`.
- `maybe_open_browser` runs before the bind succeeds.
- Personal edition with `ANCHOR_HOST=0.0.0.0` and no auth isn't gated
  (Host checks don't stop curl; `/csrf` is readable).

## Patterns (report once, fix centrally)

1. **Blocking work on the async runtime:** `extract_listed` (PDF, inline in
   `run_folder_sync`), Drive extraction in `parse_file`, `build_citations`
   and the `search` tail, `wait_for_slot`/`finish_job`, the watcher walk,
   `create_job` in the scheduler. Route through `run_db`/`spawn_blocking`.
2. **`init_db(..).unwrap_or_else(|_| Connection::open(..).unwrap())`:**
   42 copies, bypassing `with_db`; DB errors become panics.
3. **Scope filtering in Rust after retrieval instead of SQL:**
   `append_source_filter` ignores an empty set; each leg re-filters
   itself. C3 is the first leg that forgot — centralize it.
4. **Swallowed errors:** 98× `let _ = conn.execute(…)`, including vec sync,
   embedding writes, purge counts.
5. **Env read on hot paths:** 96 `std::env::var` calls, many per request
   or per chunk; makes tests order-dependent (`set_var` behind mutexes).
6. **New `reqwest::Client` per call:** 17 sites, no connection reuse.
7. **Fail-open security design:** deny-list for public paths (C1), empty
   Host allowed, trust decided by hostname string.

## Untested critical paths (tests to add)

- Auth on: every route in `build_router` without a token → 401 except an
  explicit allowlist (catches C1).
- `/qa/public` and project-scoped `/qa/search` with a query naming a file
  in a non-public source → zero hits from it (C3).
- `clean_doc_text` keeps table cells (`Yes`, `2024`, `N/A`) and strips only
  true headers/footers (H2).
- `classify_heading` negatives: `3.5 million…`, `1.0.31 release`,
  `21.10.2026`, `NOTE`, `10.0.0.1`.
- Indentation round-trip: a 2 KB Python class / YAML block reassembles to
  identical bytes; an unclosed fence >3 KB keeps its last line (M2).
- A panicking job body ends `failed` and frees its slot (H5).
- Two `create_job` calls for one source return the same job (H4).
- Jira `parse_issue` with an ADF description (M1).
- Embedder returning N−1 vectors errors instead of misaligning (M7).
- `import_handler` with a 10 MB multipart archive (M8).
- Perf guard: vector search over 50k synthetic chunks within a time budget
  (golden eval measures recall only, never cost).

Existing tests are meaningful where present (retrieval, chunking, backup).
Thin: Jira 0 tests, secrets 1, auth/security/sources/jobs 2–4 each.

## Staged action plan

### Stage 1 — Retrieval crispness, local-first (now)

Tracks A + B. Every item here changes what the golden eval and live asks
return, so pair each with a fixture in `tests/eval_data/` (fixture
discipline per its README) and keep fused recall at 1.00.

Content fidelity (A):
1. Narrow repeated-line / page-number stripping to page boundaries (H2).
   Add a table-PDF fixture whose answer lives in a repeated cell value.
2. Heading heuristic tightening + port `merge_short` to the prose path (H3).
   Fixture: release notes / numeric table that currently fragments.
3. Line-based oversize splits, whitespace verbatim; fix the unclosed-fence
   last line (M2). Byte-for-byte reassembly test.
4. Jira ADF flattening (M1).
5. Line ranges for prose/code/config chunks (M9) — improves citations.

Retrieval & answer quality (B):
6. Pass scope into the filename boost + final scope assert in
   `orchestrate_fusion` (C3).
7. nomic `search_query:` / `search_document:` prefixes + vector-count
   check (M7). Prefixes change every stored vector — ship with a
   re-embed plan (force re-sync or a backfill job), not silently. Test tail-drop + middle-drop separately; decide the re-embed plan under the R40.5 rule (note, not migration, unless recall measurably requires it — the golden eval can't see prefix effects, so measure live).
8. Answer timeout as a setting (default ≥120 s), stream, send
   `options.num_ctx`; size the context in tokens (H6).
9. Drop the 50% classify-window overlap or dedupe its entities (Low).

### Stage 2 — Local reliability & scale (alongside / right after Stage 1)

Track C. Not ranking changes, but each one is felt on a real local corpus.

10. Per-source job dedupe in `create_job` (H4 — duplicates R30.1, same fix).
11. `catch_unwind` around extraction + job supervisor guard; recover from
    mutex poisoning (H5).
12. Watcher fallback every ~10 min in `spawn_blocking` (H8 — duplicates R30.3, same fix).
13. Link sqlite-vec, or interim slim SQL-filtered scan (H1). Add the 50k
    chunk perf guard test so it can't regress.
14. Chunk outside the write tx; reuse embeddings by content hash (M3 — tx + no-migration interim = R43.12; migration-backed reuse stays R36.9).
15. Streamed, cursor-based connector sync, file size caps, Drive OAuth
    refresh — split per calibration: M4a Drive refresh is High and moves first (R43.14), M4b cursors/caps/streaming stays here (R43.13).
16. Raise the body limit on `/backup/import` (M8).
17. Clean up patterns 1, 2, 4–6: one connection helper via `run_db`, a
    per-request settings snapshot, shared `reqwest::Client`, log instead of
    `let _ =` on writes that matter.

### Stage 3 — Hosted Team: access management + security

Tracks D + E. Gate any shared/networked deployment on all of these.

Access management (D):
18. Default-deny `is_public_path` + route-walk test (C1). *Cheap enough to
    pull forward — consider doing it in Stage 1.*
19. Close signup (first user = admin, then invite-only); admin role for
    settings / sources / backup / `/system/*` (C2).
20. SPA login + Bearer handling (H7).
21. Login rate limit keyed by peer IP + email, bounded map (M6).
22. Per-data-dir keychain namespace; atomic Fernet key creation (M5).
23. Compose Ollama URL in `ANCHOR_TRUSTED_PROVIDERS` via `.env.example` (H7).

Security hardening (E):
24. SSRF egress guard on REST preview + test-connection; one-page preview (H9).
25. Escape JQL / Drive `q` values; gate personal edition on non-loopback
    bind without auth (Low).
26. Fail-closed review of the middleware stack (pattern 7).
27. `cargo audit` + `cargo outdated` in CI; plan the pdf-extract,
    rusqlite and keyring upgrades.
28. Separate review of `mcp.rs` / `mcp_setup.rs` (writes other apps'
    config files).

## Not reviewed (coverage gaps)

- Skimmed: `retrieval.rs` beyond vector / filename-boost / fusion (TOC,
  slots, `parent_return`, who-knows, graph); `answer.rs` follow-up rewrite.
- Not opened: `extract.rs` format parsers beyond PDF entry + size caps
  (DOCX/XLSX/RTF/EML internals), `vocab.rs`, `review.rs`, `entities.rs`,
  `items.rs` handlers.
- Light pass: `backup.rs` (import caps checked, merge logic not),
  `license.rs`, `mcp.rs` + `mcp_setup.rs` (writes config into other apps'
  files — deserves its own review), `skills_setup.rs`, `tray.rs`.
- Frontend: auth/token handling only.
- Dependencies: no `cargo audit` / `cargo outdated` run. Versions read
  (rusqlite 0.31, keyring 2, notify 6, pdf-extract 0.12) trail current by a
  major or more; pdf-extract is panic-prone (H5).
- Not run: conformance suite, lizard gate, release build.
