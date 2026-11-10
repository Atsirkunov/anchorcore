# AnchorCore — code review 2026-11-10

> Status: filed as Phase 50. Base `49b971b` (main, v1.7.0). Scope: Rust backend (`rust/crates/anchorcore`), React app (`frontend/`), website, scripts, CI. Legacy Python excluded. Lens: dead code, useless comments, cyclomatic complexity, overly complex methods, and the gaps found on the way. No source file was changed; this document is the only deliverable. Filed as Phase 50 (R50.1–R50.35) in `rust/BACKLOG.md`.

Paths without a directory are under `rust/crates/anchorcore/src/`. Line numbers are valid for the base commit. Each finding says how it was established: **proven** (a tool or query was run), **traced** (the code path was read end to end), or **measured** (a count from a script).

## 1. Verdict

The complexity gate works: 2,585 Rust functions, average CCN 3.2, and nothing new above 15. Complexity is not where the risk is. The risk is in four places the gates do not look at:

1. **The compiler's dead-code check is switched off for the whole crate**, so dead code accumulates unseen. A whole retrieval path (the `vec0` vector index) is dead and still runs on every query and every embed.
2. **The Team edition's login protects nothing and its UI cannot use it.** Signup is open to anyone who reaches the port, and the React app has no login screen and sends no token.
3. **Read gates are not mirrored on write endpoints**, and several read paths still turn a failure into a wrong-but-plausible answer ("nothing found", "not scoped").
4. **Comments carry the project's history instead of the code's intent**: 864 comment lines cite ticket ids, 82 point at Python files that are no longer on `main`.

The frontend is in better shape than the backend on this lens: lint is clean, there is no `any`, no inline hex colour, and one transport function.

## 2. Numbers

| Measure | Value | How |
|---|---|---|
| Rust functions / avg CCN / warnings over 15 | 2,585 / 3.2 / 5 (all in the accepted baseline) | `lizard -l rust -C 15` |
| Functions sitting exactly on the limit (CCN 15) | 6: `entity_json_from_row`, `context_handler`, `main::serve`, `store_embedded`, `create_tags_inner`, `embed_chunks` | lizard |
| Production Rust lines / in-file test lines | 34.7k / 17.8k | script |
| Items the compiler would flag with the allows removed | 12 (private items only; `pub` items in a lib are never flagged) | `cargo rustc -- --force-warn dead_code …` |
| Comment lines citing a ticket id (`R48.12`, `B57`, `F-03`) | 864 Rust, 129 frontend | script |
| Comment lines citing Python / `backend/app` / alembic | 82, of which 16 are module headers | script |
| `.unwrap()` in production code | 198 (`entities.rs` 23, `main.rs` 19, `system.rs` 14) | script |
| `filter_map(\|r\| r.ok())` / `.flatten()` on row iterators | 86 | script |
| Inline `json!({"detail": …})` error tuples / files redefining `ApiError` | 95 / 6 | grep |
| `reqwest::Client` built per call | 14 sites | grep |
| `cargo fmt --check` | 2,828 diff hunks, 360 lines over 160 columns, no `rustfmt.toml` | proven |
| Frontend: lint / `any` / inline hex / components over CCN 15 | clean / 0 / 0 / 6 (`AnswerModelStep` 31, `AgentSkillsCard` 24, `BackupCard` 19, `App` 18, `ModeStep` 17, `McpConnectCard` 16) | eslint, lizard |

## 3. Findings

Severity: **High** = wrong or unsafe behaviour a user or operator hits today. **Medium** = wrong under a condition that will occur, or a cost that grows with the library. **Low** = hygiene.

### A. Correctness and security gaps

#### CR-01 — High — Signup is open, so Team auth authenticates nobody
- **Where:** `auth.rs:289` (`signup_handler`), `auth.rs:216` (public path list).
- **What (traced):** with `ANCHOR_AUTH_SECRET` set, `POST /auth/signup` accepts any email and password and returns a 7-day bearer token. There is no first-user rule, invite, or allowlist.
- **Why it matters:** Team refuses to bind `0.0.0.0` without the secret because "Host/Origin checks authenticate nothing" (`main.rs:67`). Open signup has the same hole one request later. With the connector egress guard still open (R43.23), a network peer can sign up, add a `folder` source for any path the process can read, sync it and read it back.
- **Fix:** allow signup only while the `users` table is empty, or behind an operator-set invite secret; everything else is login. Test: second signup is 403.

#### CR-02 — High — The Team UI cannot log in
- **Where:** `frontend/src/api.ts:70` (`send`), `auth.rs:219` (comment "so the login page can load"), `hosting/Dockerfile:56-63`, `hosting/docker-compose.yml`.
- **What (traced, not run in a container):** the React app has no login screen and never sends `Authorization` (no `/auth/`, `Bearer` or 401 handling anywhere in `frontend/src`). The Team image bakes `ANCHOR_EDITION=team` and `--host 0.0.0.0`, which requires the auth secret, which turns on `require_auth_middleware` for every API route. Every call the SPA makes is then a 401. `docker-compose.yml` does not mention `ANCHOR_AUTH_SECRET`, so the documented `docker compose up` refuses to boot until the operator adds it — after which the UI is unusable.
- **Why it was missed:** CI's Team smoke boots with an edition override and only checks `/health` (`ci.yml:186`).
- **Fix:** a login view plus token storage and a 401 handler in `send()`; add a CI step that boots Team with the secret and loads one authenticated route. Confirm in a container first.

#### CR-03 — Medium — Write endpoints return the bytes the read gate withholds
- **Where:** `items.rs:684` (`PATCH /items/:id` returns `fetch_item`: full text, content, assets), `entities.rs:188` and `:269` (`PATCH /entities/:id`, `POST /entities/:id/dispute` return `window_text`, `summary`, `reasoning`).
- **What (traced):** none of the three builds a `read_gate::Gate`. `PATCH /items/:id` with `{}` writes nothing and returns the whole document. A gated caller that gets 403 `gated_provider` from `GET /items/:id` gets the same bytes from the PATCH. The same endpoint lets any caller clear a person's Sensitive mark with `{"sensitive_override": null}`, although AGENTS.md calls the override escalate-only.
- **Fix:** run the same `check_bytes` on the response (or return metadata only to gated callers), and accept `null` only from the UI caller, like `PUT /settings/sensitive-sharing`.

#### CR-04 — Medium — Jira "load projects" always fails
- **Where:** `routes.rs:25` → `stubs.rs` (501 "Not Implemented (Rust stub R1.3)"); `connectors/jira.rs:169` (`list_projects`, complete and never called); `frontend/src/tabs/sources/SourceForm.tsx:57`.
- **What (traced):** the Jira source form calls `POST /sources/jira/projects`, which is the last surviving port stub. The implementation exists in the connector.
- **Fix:** a ten-line handler that calls `JiraConnector::list_projects`; delete `stubs.rs`.

#### CR-05 — Medium — A question with no match records two false warnings
- **Where:** `answer.rs:238` (`record_degraded`), called only at `:629` when `hits.is_empty()`.
- **What (traced):** it inserts "retrieval degraded to keyword-only" and "answer generation failed; returning matching context only". Neither happened. Every miss adds two warning rows to `system_events`.
- **Related:** `answer.rs:626` and `:1033` use `.unwrap_or_default()` on the retrieval task, so a panic inside retrieval is reported as "No relevant knowledge found" (plus the two rows above). `public_source_ids` (`:878`) has two `unwrap`s on that path.
- **Fix:** delete `record_degraded` (the response already carries `mode` and `retrieval`); map a failed retrieval task to an error.

#### CR-06 — Medium — Scope filters fail open
- **Where:** `answer.rs:870-875` (`project_source_ids`), `retrieval.rs:381` (`scope_hit_ok`), `retrieval.rs:557-563` (`make_hit`).
- **What (traced):** a failed scope query returns `None`, which means "no project filter", so the search runs over every source. A hit whose `source_id` is NULL passes any project or `public_only` scope.
- **Fix:** return `Result` from the scope lookup; treat a source-less hit as out of scope whenever a scope is set. This is the R48.11 rule (gate inputs fail closed) applied to scoping.

#### CR-07 — Medium — A migration error that says "already exists" stops migrating and reports success
- **Where:** `db.rs:418-426` (`apply_pending`).
- **What (traced):** the tolerance was written for databases created by the Python build. Today any migration whose error text contains "already exists" or "duplicate column" logs a warning, returns `Ok`, skips every later version, and the path is cached as migrated. A future `CREATE INDEX` without `IF NOT EXISTS` would be swallowed on affected databases at every boot.
- **Fix:** apply the tolerance only when `user_version` is 0 and the legacy tables exist; otherwise fail the boot as designed in R41.7. Error-string matching for control flow also appears at `db.rs:182`, `db.rs:337`, `embedder.rs:474`.

#### CR-08 — Medium — Log download returns the oldest 512 KB
- **Where:** `system.rs:287-294`.
- **What (traced):** the handler reads the whole file (up to 10 MB) and keeps `data[..MAX_LOG_BYTES]`. The newest lines, which a bug report needs, are dropped.
- **Fix:** seek to `len - cap` and serve the tail from the next line break. The `is_log` name check is written twice (`:250`, `:277`).

#### CR-09 — Medium — The "bundled" sample corpus is not bundled
- **Where:** `system.rs:191-208`.
- **What (traced):** the sample folder is looked up at `env!("CARGO_MANIFEST_DIR")/../../../sample` (the build machine's absolute path, compiled into the release binary), `./sample`, and `{data_dir}/../sample`. No build script or release job copies `sample/`. Installed users never see "Try the sample corpus" in the wizard. The three paths are also computed twice in the same function.
- **Fix:** embed `sample/` with `include_dir!` (as `frontend/dist` is) and materialise it under the data dir on request.

#### CR-10 — Medium — Only Jira and folder sources are ever polled
- **Where:** `scheduler.rs:89-93` (`_ => continue`), `scheduler.rs:108-112`.
- **What (traced):** Drive, Linear, Miro and REST sources sync only on a manual click. The two intervals are read from `jira_poll_minutes` and `folder_scan_minutes`, which are not in `SETTING_KEYS`, so they are environment-only and absent from the UI. Any source create, update or delete aborts and respawns every poll task, and each restarts its interval from zero.
- **Fix:** decide per connector (owner call), put the interval where a person can see it, and reload only the changed source.

#### CR-11 — Medium — `/system/status` returns invented fields and the UI renders them
- **Where:** `system.rs:99-137`; `frontend/src/tabs/settings/StatusSection.tsx:166-171`, `:195`, `:250-260`; `frontend/src/types.ts:106-113`.
- **What (traced):** `"retrieval": {"calls": 0, "vec0_calls": 0, "avg_latency_ms": 0.0, "backend": "vec0"}`, `"tasks": {}`, `"data_dir": "redacted"`, `"database": "redacted"` are constants. The backend is not vec0. The retrieval block and `StoppedTasks` component can never show.
- **Fix:** remove the fields, the two UI blocks and the types.

#### CR-12 — Medium — `PATCH /entities/:id` accepts any `kind` and `status`
- **Where:** `entities.rs:199`.
- **What (traced):** both are free strings. Any caller can write `status = "verified"` — which the owner decision defines as "a person confirmed it" — or a value `status_ok` does not know.
- **Fix:** enums with a 422, and restrict `verified` to the UI caller.

#### CR-13 — Medium — Auth hardening gaps
All in `auth.rs` and `security.rs`, traced:
- The limiter key is the email (`auth.rs:96`), so five bad logins lock a known user out for 60 s, repeatably. The map prunes only on same-key access, so random emails grow it without bound; the comment at `:38` says it is bounded.
- `require_auth_middleware` returns a bare status with no `{"detail"}` body (`auth.rs:236`), opens the database through `db::init_db` on every request, and `me_handler` repeats its token parsing and user lookup (`:393-418`).
- `hash_password` and `verify_password` (PBKDF2, 200k rounds) run on the async runtime (`:311`, `:377`).
- `decode_token` keeps a hand-rolled constant-time loop (`:185-194`); R48.16 added `common::ct_eq` for this.
- `ANCHOR_CSRF_DISABLE=1` (`security.rs:218`) turns CSRF off in production builds. Its only consumer was the archived Python test harness.
- `allowed_hosts_set` and `allowed_origin_prefixes` re-read the environment, take a mutex and clone a set on every request; host and port parsing is hand-written three times in `security.rs` and once more in `common::provider_is_local`.

#### CR-14 — Low — Test-fixture synonyms ship in production tag matching
- **Where:** `embedder.rs:504-515`, used by `tags.rs:56`.
- **What (traced):** `deterministic_embed` maps `car/vehicle`, `database/db/storage/store`, `red/crimson → "color"` and others to one token. The comment says "test-only probe helper… not production mixing", but tag reuse runs on these vectors, so tags that differ only by those words merge.
- **Fix:** move the map into the tests that need it.

#### CR-15 — Low — Two of three embedding writers ignore a failed family stamp
- **Where:** `pipeline.rs:1942` and `scheduler.rs:314` (`let _ = embedder.sync_vec(..)`), against `pipeline.rs:1992` (`?`, with the comment "a vector without its family stamp is never searched — both or an error").
- **Fix:** one `write_embeddings(conn, embedder, updates)` used by all three.

#### CR-16 — Low — Smaller gaps
- `settings.rs:117-136`: `dotenvy::dotenv()` runs on every lookup that misses the environment (the comment says "loaded once"; the `ENV_CACHE` meant for it at `:115` is dead). `settings.rs:333`: a failed database open in `cached_get` silently yields defaults for 3 s.
- `sources.rs:177`: `connector` is any string at create time; a bad kind fails only at the first sync.
- `pipeline.rs:61`: the variable `redacted` is only truncated to 1,000 characters before it is stored in `sources.last_error` and `system_events`.
- `entities.rs:57`: the `item_sensitive` display flag reads `.unwrap_or(false)` (a missing item reads as not Sensitive). The gate itself is correct.
- `jobs.rs:312`: a database error in `cancel` reads as "not cancellable" and the handler answers 409 "Job #n is running, not running".
- `backup.rs:2448-2462`: an upload already saved to `exports/` is left behind when a later multipart field fails.
- `embedder.rs:364`: `embed_timeout as u64` drops fractions (0.5 s becomes 0), while `answer_timeout` honours them.
- `watcher/service.rs:220`: one-second sleep per changed source inside the loop.

### B. Dead code

#### CR-20 — High — Dead-code detection is disabled crate-wide
- **Where:** `lib.rs:6` and `main.rs:1` (`#![allow(dead_code)]`), `mcp.rs:1` (`#![allow(dead_code, unused_imports, unused_variables)]`), plus `settings.rs:114`, `system.rs:232`, `watcher/service.rs:6,8,17,287`.
- **What (proven):** with the lints forced on, the compiler reports the twelve items below. Because every module is `pub mod` in a library crate, unused `pub` items are never reported at all; those were found by reference counting.
- **Fix:** delete the items in CR-21, remove the three crate-level allows, and narrow visibility to `pub(crate)` so the compiler sees the rest. The `mcp.rs` allow currently hides one field.

#### CR-21 — Dead items (all verified by reference search)

| Item | Where | Note |
|---|---|---|
| `tags_for_chunk` | `answer.rs:287` | predecessor of the batched lookup |
| `settings_snapshot`, `_settings_map`, `_settings_clone` | `answer.rs:783`, `:861`, `:1020` | built on every ask and search (four settings reads, one from the secret store), read by nothing |
| `split_section` | `chunking.rs:1013` | wrapper never called |
| `fuse_evidence` | `retrieval.rs:1464` | `pub`, no caller |
| `retrieval::cosine`, `retrieval::unpack_f32`, `env_for_path` | `retrieval.rs:388`, `:401`, `:2037` | tests only; `embedder.rs` has its own copies |
| `Embedder::provider_is_local`, `pack_f32` | `embedder.rs:311`, `:75` | never used / tests only |
| `items::list_items` | `items.rs:394` | tests only; production calls `list_items_filtered` |
| `ENV_CACHE` | `settings.rs:115` | never used |
| `LogFileOut`, field `answer_api_key`, wrapper `provider_is_local` | `system.rs:234`, `:37`, `:14` | never constructed / read |
| `UpgradePlan.of` | `db.rs:73` | never read |
| `JobManager.pending`, `.running`, `Job`, `JobStatus`, `Job::from_row`, `JobManager::list`, `::running`, `Default` impl | `jobs.rs:192-363` | the maps are written and never read; `pipeline.rs:642` proves it by calling `JobManager::new().maybe_promote(..)` on a throwaway instance. About 70 lines |
| `KINDS`, `BUCKETS` | `classifier.rs:22`, `:24` | unused / tests only |
| `JiraConnector::list_projects` + `stubs.rs` | `connectors/jira.rs:169` | see CR-04 |
| `common::provider_trusted` | `common.rs:141` | 11 test assertions, no production caller; `sharing::allowed` is the rule |
| `extract::is_code_path` | `extract.rs:118` | tests only |
| `WatcherService` (struct, `new`, two `spawn`s, `reload`) | `watcher/service.rs:18-49` | `handles` is never filled; `reload` logs "aborted {} handles" after draining, so always 0; nobody calls it |
| `SourceSchedule.enabled`, `.last_synced_at` | `scheduler.rs:25-27` | always `true` / `None` |
| Unused parameters | `pipeline.rs:925` (`_force`), `retrieval.rs:1053` (`_source_ids`), `mcp_setup.rs:431` (`_exe`) | |
| `pipeline.rs:142` | unused variable `out` | |
| Frontend: `ItemImages`, `api.defaultProject`, `api.sectionChunks`, `api.tagChunks` | `ImageStrip.tsx:102`, `api.ts:194`, `:366`, `:368` | no caller |
| Frontend: FastAPI-style `detail` array branch | `api.ts:98-100` | the Rust server never returns it |
| Repo root: `palette-preview.html`, `palette-preview-c2.html` | tracked | scratch pages |

#### CR-22 — High — The `vec0` vector path is dead, runs on every query, and doubles vector storage
- **Where:** `db.rs:324-375` (`ensure_vec`), `embedder.rs:452-481` (`sync_vec`), `retrieval.rs:434-448`, `:528-534`, `:592-668`, `:759-768`.
- **What (proven):** no sqlite-vec is linked, so `vec_chunks` is a plain table. Running the vec0 query against that shape fails at prepare with `no such column: v.distance`. Consequences:
  - **Per query:** each vector leg (up to four per question) serialises the 768-float query to JSON, looks up the comparable fingerprints, prepares a statement that cannot succeed, then falls through to the scan.
  - **Per embed:** the fallback triggers copy each vector blob into `vec_chunks`, then `sync_vec` unpacks it, serialises it to JSON text (roughly 10 KB against a 3 KB blob; estimated from the float format, not measured on a library) and replaces the row. The only readers of the table are `vec_chunk_count` and the failing query; the scan reads `chunks.embedding`. Every vector is stored a second time, larger than the original, and nothing reads it.
  - **Per boot:** `ensure_vec` tries `CREATE VIRTUAL TABLE … USING vec0`; its success branch (`:357-374`) is unreachable.
- **Fix:** drop `vec_chunks` and its triggers in a migration, reduce `sync_vec` to the fingerprint stamp (and rename it), delete the vec0 search path and `KindScope::kind_clause`'s always-`Some` option. About 200 lines go. If sqlite-vec is linked later (R43.10), it returns as one module.

#### CR-23 — Medium — `mod suspect_pdf` duplicates the PDF detector for a reason that no longer holds
- **Where:** `retrieval.rs:1216-1327`, a copy of `extract.rs:697-810` (about 110 lines).
- **What (traced):** its comment says the copy exists because the eval target compiled `retrieval.rs` standalone through `#[path]`. `lib.rs:1-4` says that arrangement is gone.
- **Fix:** delete the module and call `crate::extract`.

### C. Performance and storage

#### CR-30 — High — `ingested_items` has no index and no uniqueness on `(source_id, external_id)`
- **Where:** every `CREATE INDEX` in `migrations/` was listed (proven); the table has only `content_hash` and `uid` indexes.
- **What:** the lookups `WHERE source_id = ? AND external_id = ?` run several times per document during a sync (`pipeline.rs:929`, `:965`, `:967`, `:1091`, `:914`, `:218`) and each scans the table, so sync cost grows with files × items. Nothing but the one-job-per-source rule prevents two rows for the same file, and `upsert_doc` turns a failed lookup into an insert (`.ok()` at `:929`). The foreign key `source_id` is also unindexed, against the AGENTS.md rule from R41.8.
- **Fix:** a migration that removes duplicates and adds `UNIQUE INDEX (source_id, external_id)`; extend the v10-sim test.

#### CR-31 — Medium — The PDF "suspect" check re-reads every PDF's full text on each question
- **Where:** `retrieval.rs:1334` (`suspect_pdf_items`, called from `toc_search`), `items.rs:523` (`pdf_flags_for_page`).
- **What (traced):** on the TOC path each query selects the stored text of every in-scope PDF and runs the split-word detector over it. Nothing is cached or stored. `GET /items` does the same for every PDF row on the page (up to 1,000) and calls `canonicalize` once per row rather than once per source.
- **Also:** when suspects exist, `append_suspect_flat_legs` runs the unfenced vector and keyword legs, and `orchestrate_fusion` then runs the same unfenced legs again as the recall floor.
- **Fix:** compute the verdict at ingest and store it on the item (one column, one migration); pass the flat-leg results into the floor.

#### CR-32 — Medium — Database work on the async runtime
- **Where:** `answer.rs:1044-1090` (search opens a connection and runs nine queries inline), `answer.rs:663` and `:755` (`build_citations`), `pipeline.rs:544` (`finish_guarded`), `scheduler.rs:131-144`, `backup.rs:2405`, `mcp.rs:205`, `auth.rs:311` and `:377`.
- **What (traced):** AGENTS.md requires `spawn_blocking` for database work. These call sites run it on runtime workers. `wait_for_slot` is the known case (R48.23).

#### CR-33 — Low — No connection reuse
Every `run_db` opens a new SQLite connection (open, three PRAGMAs, a mutex). A sync makes about ten such calls per document, including one cancel check and one progress write per file. `settings.rs:333` opens one per key per three-second cache miss. Worth a per-job connection before a pool.

### D. Complexity and structure

The gate holds, so this section is about what the gate cannot see.

#### CR-40 — Medium — The search hit is described in five places
`answer.rs` has `Citation` and `SearchHit` with about 22 identical fields, and two 35-line mappers (`:544`, `:1056`) that build the same nine lookup maps. `mcp.rs` projects both again by hand (`citation_json`, `search_hit_json`), and `frontend/src/types.ts` mirrors them. A new field means five edits. **Fix:** one `HitView` struct with `#[serde(flatten)]`, one mapper.

#### CR-41 — Medium — Ten copy-pasted batch lookups
`answer.rs:292-542` and `:1162-1231`: `tags_for_chunks`, `conflicts_for_entities`, `entities_for_items`, `doc_types_for_chunks`, `item_meta_for_chunks`, `section_breadcrumbs`, `line_ranges_for_chunks`, `kinds_for_chunks`, `item_paths_for_chunks`, `section_titles_for_chunks` — about 250 lines with one shape (prepare error → empty map, `rows.flatten()`). Five of them read the same `chunks ⋈ ingested_items` rows, so a response costs nine queries where two or three would do. **Fix:** one generic `batch_map` and one combined chunk-metadata query.

#### CR-42 — Medium — Positional parameters instead of types
- `orchestrate_fusion` and `fuse_and_rank` take nine positional arguments each under `#[allow(clippy::too_many_arguments)]`; `answer::orchestrate` exists only to unpack `RetrievalConfig` into them. Pass the struct.
- `make_hit` takes twelve; `HitRow10` is an eleven-element tuple with two columns selected and discarded.
- The sync result `(i64, i64, i64, i64)` (items, entities, purged, unchanged) is threaded through six signatures; `update_sync_progress` takes seven positional numbers. Use a `SyncCounts` struct.
- `Err("job cancelled".to_string())` appears at eleven sites and is matched by string equality in `finish_job` (`pipeline.rs:683`). Use an error enum.
- `fuse_and_rank` (`retrieval.rs:1524-1540`) tests `is_none()` and then calls `unwrap()`; a `match` on the pair removes both.

#### CR-43 — Low — Functions kept at the limit by being left alone
`main::serve` (100 lines, CCN 15) reads `resolve_host` four times, `running_in_container` twice, the edition twice, and repeats the log-print-exit block three times. `context_handler` (CCN 15) and `expand_context` (13) re-chunk the whole document with the plain chunker on every request to find neighbours, although the stored chunks exist and Markdown, code and config items were chunked by other splitters. `embed_chunks` (15) and `run_embed_missing` share their tail (see CR-15).

#### CR-44 — Low — Repeated wiring and idioms
- The three lines that build a `Pipeline` appear in `sources.rs:424`, `scheduler.rs:151`, `backup.rs:2415`, `watcher/service.rs:277`; two of them build a new `Embedder` instead of using `AppState.embedder`.
- `SecretStore::new(..join("secrets.enc"))` is constructed per call in `sources.rs` (once per row in the list handler), `pipeline.rs:28`, `backup.rs`; the key `source:{id}:{field}` is hand-formatted in three files although `secrets::source_secret_key` exists.
- Three transaction idioms: `unchecked_transaction` (5 files), hand-written `BEGIN IMMEDIATE`/`COMMIT` (9 sites), and `jobs::in_immediate_tx` (private).
- `job_json` is identical in `sources.rs:90` and `jobs.rs:367`; the 11-column jobs `SELECT` is written four times, the sources one three times.
- The gated-row filter loop is identical in `entities.rs:140-152`, `entities.rs:241-253` and `tags.rs:350-360`; `records_api.rs:214` is a `retain` variant of it. A `Gate::partition` helper removes all four.
- `match result { Ok(v) => Ok(Json(v)), Err(e) => Err(e) }` appears five times; it is `result.map(Json)`.
- Defaults are defined in `settings::env_value` and again as 47 scattered `unwrap_or_else` literals (`localhost:11434` ×15, `nomic-embed-text` ×14, `api.openai.com/v1` ×12, `gpt-4o-mini` ×6) that can never be reached. `ALL_KEYS` is `SETTING_KEYS` plus `SECRET_KEYS` typed out by hand.
- The answer-provider label (`ollama` / `configured` / `missing`) is derived in `system.rs:51`, `system.rs:183` and `health.rs:64`. The Ollama launch block is in `main.rs:449` and `system.rs:360`. `panic_message` is in `common.rs:44` and again in `main.rs:128`.
- Retrieval knobs are read from the environment inside about fifteen functions per query; `ANCHOR_EMBED_DIM` is parsed in five files. One config struct read at startup would make them testable.
- Test setup (`SecretStore` + `SettingsService` + `Embedder`) is hand-copied in about sixty tests across fifteen files.

#### CR-45 — Low — File sizes
`retrieval.rs` is 5,581 lines (2,828 production), `pipeline.rs` 3,851, `answer.rs` 3,350, `backup.rs` 3,345. In-file test modules account for a third of the crate. Splitting `retrieval.rs` into legs, fusion, return-shape and env/authority modules would make the dead path in CR-22 obvious at a glance.

### E. Comments

#### CR-50 — Medium — Comments record history, not intent
- **Ticket ids (measured):** 864 comment lines in Rust carry one (`retrieval.rs` 141, `answer.rs` 83, `pipeline.rs` 82, `db.rs` 51). AGENTS.md keeps phase history out of the guide; the code is where it went. A reader has to skip the id to find the reason, and the id only resolves through `BACKLOG-archive.md`.
- **Dead references (measured):** 82 lines cite Python files, `backend/app/…` paths or alembic revisions. Sixteen module headers begin "port of `backend/app/x.py`". `main.rs:8` points at `docs/rust-port.md:1` and `sources.rs:427` at `docs/handover-2026-08-20-sync.md`; both moved to `docs/archive/`.
- **Narration of removed code:** 77 lines of the form "used to", "was dead code", "retired", "is gone" — for example `retrieval.rs:1542` ("the R32.3 ×0.5 multiplier retired…"), `retrieval.rs:289` (a comment about a field that no longer exists), `scheduler.rs:46`, `main.rs:420`.
- **False statements:** `routes.rs:19` ("stub all routers with 501"), `sources.rs:170` ("minimal create for tests" on the production create), `mcp.rs:2` ("6 read-only tools", there are 12; the CI step name repeats it), `mcp.rs:6` ("single DB owner", while `audit_mcp` writes to SQLite from the MCP process), `settings.rs:130` ("loaded once"), `auth.rs:38` ("to bound map"), `embedder.rs:503` ("test-only… not production mixing"), `main.rs:461` ("will use rule fallback").
- **Thinking aloud:** `settings.rs:419-421`, `security.rs:181`, `security.rs:206`, `mcp.rs:177`.
- **Ticket ids in user-visible log lines:** `db.rs:145`, `:491`, `:648`, `:697`.
- **Fix:** one pass per file. Keep the sentence that states a constraint; drop the id, the Python pointer and the story. A commit message is the place for the story.

#### CR-51 — Low — Doc comments attached to the wrong item
Five places where two doc blocks merged and the first now documents its neighbour: `common.rs:112-124` (the `provider_trusted` text sits on `provider_is_local`), `settings.rs:156-162` (the `non_empty` text sits on `is_stored_key`), `items.rs:368-377` (the listing doc sits on `struct ListFilter`), `items.rs:517-520` (on `type ListRow`), `answer.rs:315-316` (the `doc_types_for_chunks` text sits above `conflicts_for_entities`). `backup.rs:2343-2349` has the same banner twice.

### F. Tooling and gates

#### CR-60 — Medium — Three of the stated gates are not enforced
- **Clippy cannot fail:** `ci.yml:66` ends with `|| echo "clippy warnings allowed for now (R11.3)"`, and passes `-D clippy::unwrap_used` to a crate that allows it at the root.
- **The complexity gate is manual:** no CI step runs lizard, for Rust or for the frontend (six components are over 15).
- **No format check:** there is no `rustfmt.toml` and `cargo fmt --check` reports 2,828 hunks. Hand-formatted blocks such as `retrieval.rs:1464-1468` and `:1548-1557` are the visible result.
- **Fix:** make clippy fail on a short explicit lint list, add the lizard command from `rust/AGENTS.md` as a step, and adopt rustfmt in one mechanical commit (with a `max_width` that matches the current style) before adding the check.

#### CR-61 — Low — Largest untested components and paths
Frontend components without a test file: `SourceForm` (389 lines), `SettingsForm` (388), `App` (370), `StatusSection` (310), `AskTab` (296), `RecordsViews` (280). In Rust, `WatchLoop` and `trigger_sync` are compiled out under `cfg(test)`, so the watcher's reconcile and trigger wiring has no unit coverage.

### G. Frontend, website, scripts

- `frontend/src/api.ts`: 19 inline `import("./types").X` annotations next to a 600-character import line; `StatusSection.tsx` dead blocks (CR-11). The app has 24 `catch` blocks that swallow the error; the ones sampled (`api.ts:35`, `:103`, `themeMode.ts`) are commented and deliberate, the rest were not read.
- `website/src/components/AppSession.ts` and `ClaudeSession.ts`: two `play()` functions at CCN 22 and 23 with the same structure. One shared player.
- `scripts/sync_version.py` and `rust/scripts/sync_version.py` are the same script with a different path line; CI runs both to catch drift. Keep one and call it from both places.
- `rust/scripts/bench_retrieval.py` benchmarks the vec0 path and a `RetrievalTracker` that no longer exist (see CR-22); delete it with that path.

## 4. Already filed and still open

Not repeated above; several findings lean on them.

| Row | Subject | Relation |
|---|---|---|
| R48.19 | byte-index slicing of untrusted error text (`mcp.rs:199`, connectors) | still present |
| R48.20, R48.21 | export drops undecodable rows; import conflates errors with absence | `db.rs:163` has the same `let _ = backfill_uids` at boot |
| R48.22 | folder listing errors read as deletes | — |
| R48.23 | `wait_for_slot` polls on the runtime | CR-32 |
| R48.24 | watcher `trigger_sync` failures are silent | the scheduler tick has the same two silent `continue`s |
| R43.10 | link sqlite-vec | CR-22 is the cleanup if it is not linked |
| R43.16 | shared `reqwest::Client`, settings snapshot | 14 client build sites remain |
| R43.23 | connector egress guard | raises CR-01 |

## 5. Suggested order

1. **CR-01, CR-02** — Team auth. Confirm CR-02 in a container first. Small, and the only findings that affect a paid edition.
2. **CR-20, CR-21, CR-22, CR-23** — turn the dead-code lint back on and delete what it and the reference search found. One commit for the vec0 path with its migration; one for the rest. Removes about 600 lines and the duplicate copy of every vector.
3. **CR-30** — the unique index. One migration.
4. **CR-03, CR-05, CR-06, CR-04** — gate on write responses, false warnings, scope fail-open, Jira stub. Each is under an hour with a test.
5. **CR-60** — make the three gates real, so steps 2 and 4 stay done.
6. **CR-31, CR-07, CR-08, CR-09, CR-11** — as time allows.
7. **CR-50, CR-51** — comment pass, file by file, alongside whatever touches the file next. Do not run it as one sweep across files other sessions are editing.
8. **CR-40 to CR-44** — structural duplication, only together with a change that needs it.

Owner calls: CR-10 (which connectors poll), CR-12 (who may set `verified`), CR-03 (who may clear a Sensitive mark).

## 6. What was read

- **Read in full (production part):** `common`, `routes`, `main`, `db`, `settings`, `system`, `sources`, `jobs`, `entities`, `items`, `read_gate`, `security`, `auth`, `answer`, `scheduler`, `watcher/service`, `embedder`, `stubs`, `lib`; `frontend/src/api.ts` transport.
- **Read in part, rest outlined by lizard:** `retrieval` (search core, TOC, fusion, orchestration — about half), `pipeline` (sync, classify context, chunk and embed writes — about two thirds), `mcp` (proxy core), `backup` (handlers and `import_archive`).
- **Scanned by tool only** (lizard, forced compiler warnings, reference counts, pattern counts): `extract`, `chunking`, `markdown`, `config_chunks`, `diagram`, `diagram_formats`, `assets`, `captions`, `records`, `records_api`, `sections`, `tags`, `vocab`, `sharing`, `events_api`, `projects`, `glossary`, `report`, `startup`, `tray`, `license`, `cli`, `mcp_setup`, `skills_setup`, `ollama`, `llm_chat`, `model_store`, `embed_local`, `embed_fp`, `connectors/*`, the frontend components, the website, and the test suites. Findings in these files are limited to what the tools show; they did not get a line-by-line read.
- **Not run:** the test suite, the Team container, a live sync. CR-02 in particular should be reproduced before work starts.
