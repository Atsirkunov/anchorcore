# AnchorCore Rust Port — Tech Lead Review 2026-08-21 (Lead / Handover)

**Reviewer:** seasoned tech lead (external) — Rust port only (`rust/`), Python ignored except as contract  
**Branch/commit:** `main` @ `9ea85ed` (R6.5–R6.6) / `9eaa9a4` (R6.1–R6.4 batch), workspace `rust/Cargo.toml:1` `1.0.8`  
**Scope files:** `docs/port-review-2026-08-21.md:1` + `docs/handover-2026-08-21.md:1` + `rust/BACKLOG.md:1` + `docs/rust-port.md:1` + full `rust/crates/anchorcore/src/**` (26 modules, ~3,700 LOC)  
**Signals at review time:** `cargo test -p anchorcore` 37 passed / 0 failed (5.27s), `cargo check -p anchorcore` 71 warnings, `PYTHONPATH=backend ANCHOR_TEST_RUST_URL=http://127.0.0.1:8123 pytest -q --ignore=test_mcp.py` reported 124 passed / 3 skipped (86s, fresh `/tmp/ac-full-test.db`). See `docs/port-review-2026-08-21.md:70`.

> **TL;DR:** Contract-first incremental is the right call and ~60% of the port is solid. The “124 green” cutover signal is **inflated** — vector embeddings are never written, `graph_expand`/`who_knows`/planner still run through Python’s `AnswerEngine` on the shared DB, and `main.rs` now carries a 130-line watcher state machine that owns DB, scheduler, and secrets. Do not retire Python until the P0s below land. This doc is the handover checklist for the next agent/owner.

---

## 1. Methodology

* Read `docs/rust-port.md:1` eval (why Rust), `rust/BACKLOG.md:1` phases, `docs/handover-2026-08-21.md:1` what landed in R6.1–R6.4, `docs/port-review-2026-08-21.md:1` prior optimistic summary.
* Inspected every `rust/crates/anchorcore/src/*.rs`, `src/connectors/*.rs`, `src/bin/mcp.rs:1`, `migrations/*.sql`, `rust/Cargo.toml:1`, `rust/crates/anchorcore/Cargo.toml:1`, plus `AGENTS.md:1` + `rust/AGENTS.md:1`.
* Ran `cargo test` / `cargo check` in `rust/` to ground claims; did not need to re-run full Python harness — prior artifact `124/3` taken as reported, but down-scoped (see §3).
* Rated findings P0 (blocks cutover), P1 (high — ship debt), P2 (medium — design debt). Every finding cites `file:line`.

---

## 2. Architecture snapshot (as-built)

```
frontend/dist (include_dir) ─┐
                             ├─ axum 0.7 Router (main.rs:240) ── fallback frontend::handler
reqwest (Ollama/cloud) ──────┤                            ├─ /health, /sources, /entities+context, /review, /pii, /projects
                             │                            ├─ /qa, /qa/public, /qa/search (answer::ask) ── retrieval::*
                             │                            ├─ /settings, /system/*, /auth/*
                             │                            └─ /sources/:id/sync → jobs::JobManager → pipeline::Pipeline ── connectors/*
                             │
                      AppState{data_dir, settings:Arc<SettingsService>, jobs:Arc<JobManager>} (health.rs:8)
                             │
DB: rusqlite 0.31 bundled, rusqlite_migration, PRAGMA foreign_keys=ON/WAL/busy_timeout=5s (db.rs:25)
    migrations 01..10 (collapsed Alembic) + ensure_columns + ensure_vec (dummy vec_chunks if vec0 missing)
    SecretStore per-key fernet file (secrets.rs:46) + keyring fallback
    MCP sidecar (bin/mcp.rs:1) stdio → http_call → backend 127.0.0.1
```

Parallel-agent plan (`rust/BACKLOG.md:1`) file-sharded: R6.1 `settings.rs:94` / R6.2 `system.rs:1` / R6.3 `auth.rs:1` are disjoint, R6.4 `pipeline.rs:1` vs R6.5 `retrieval.rs:1` disjoint. That part is well-designed.

---

## 3. Critical gaps — P0 (block cutover)

### 3.1 “124 green” is half-Python — Rust retrieval hardening not exercised

`docs/handover-2026-08-21.md:46` admits: `status_ok`/`graph_expand`/`who_knows`/planner tests use **Python’s `AnswerEngine` directly on the shared DB**, not Rust HTTP. `rust/BACKLOG.md:75` marks R6.5 “done” while noting “already green via Python AnswerEngine + Rust source_ref fix”. So fused ranking bugs, disputed gate, HOPS weighting, window context never gate the suite.

*Evidence:* `answer.rs:222` `retrieve_sync` hardcodes `fuse_and_rank(conn, None, Some(evidence),…)` with `vector_hits=None`; `retrieval.rs:582 fuse_and_rank` only exercised for `keyword_hits` → `keyword_fallback`. `graph_expand` path (`answer.rs:232`, `retrieval.rs:511`) runs but asserts cite Python, not Rust `Hit.source_ref` path.

**Impact:** false confidence for Phase 6. A later `vec0` regression would still be 124 green.

**Fix (next agent):** add Rust-only conformance mode — force `test_graph_retrieval`, `test_planner`, `test_disputes` via `ANCHOR_TEST_RUST_URL` (or new `rust/tests/retrieval_harness.rs` that seeds DB with `rusqlite` and calls `retrieval::graph_expand`/`who_knows_search`/`fuse_and_rank` directly). Gate cutover on that.

### 3.2 Embeddings never written — vec0 path is dead code

`pipeline.rs:428-453`:
```rust
let chunks: Vec<(i64,String,bool)> = ... // SELECT id, content, is_pii FROM chunks WHERE …
let pending … // TODO
let is_pii_flags …
let corpus …
let mut embeddable_idx = vec![];
for (i,c) in contents.iter().enumerate() {
  if signal(c,&corpus) >= embed_min_signal { embeddable_idx.push(i); }
}
// For tests … skip actual embed call … do nothing
Ok(total_entities)
```
`embedder.rs:114 embed_gated()` exists but is **never called** from pipeline. `db.rs:94 ensure_vec` creates dummy `vec_chunks(rowid,embedding BLOB)` fallback so `test_vec_chunks_table_exists` passes without `sqlite_vec`. `system.rs:73 retrieval{backend:"vec0", vec0_calls:0}` is literal. `answer.rs:222` never passes `vector_hits`; `vector_search_vec0:176` requires `query_emb.len()==dim` and `vec_chunks USING vec0(embedding float[768])` which never sees rows.

**Impact:** retrieval benchmark in `docs/rust-port.md:1` (vec0 8–12ms vs scan 45–90ms) never validated on Rust; performance win of port unproven. Also `signal` IDF gate tested in isolation (`distill.rs:13`) but never gates real writes.

**Fix:** wire in `classify_and_store:428`:
```rust
let embedder = self.embedder.clone();
let gated = sensitive_label(&label);
let out = embedder.embed_gated(contents.clone(), is_pii_flags, gated, embed_min_signal).await?;
for (id, blob) in pending_ids.iter().zip(out) {
  if let Some(b)=blob { conn.execute("UPDATE chunks SET embedding=?1 WHERE id=?2", params![b, id])?; embedder.sync_vec(&conn, *id, &b)?; }
}
```
Add `pack_single`/`unpack_f32` roundtrip + `ANCHOR_EMBED_MIN_SIGNAL` property test. Flip `system::status_handler` to report real `vec0_calls` (counter + `Instrumentation`).

### 3.3 `main.rs` watcher monolith — 130 LOC God task

`main.rs:107-235` spawns one `tokio::spawn(async move { loop { // discover folder sources via spawn_blocking // ensure watchers // poll 300ms // to_sync sleep 1s // fallback fetch-compare every loop }})` that:

* Opens DB via `init_db` **2× per second per source** (discovery + config read + fallback) — each `init_db:25` does WAL/pragma + 10 migrations `to_latest` + `ensure_columns` + `ensure_vec` triggers + backfill. At 11 watchers (stress repro cited in `docs/handover-2026-08-21.md:49`) that’s ~22 full migration scans/s.
* Re-creates `Classifier`/`Embedder` (`main.rs:185`) per file event — drops `SettingsService` cache (3s TTL, `settings.rs:36`).
* Maintains duplicate sync logic: `watcher.poll(300ms)` + “fallback fetch-compare even if poll empty, every 2nd loop” (`main.rs:194`) that double-queues on slow FS (FSEvents coalescing noted correctly but fix doubles writes).
* Has two names for DB URL (`ANCHOR_DATABASE_URL` at `main.rs:56` vs `ANCHOR_DATABASE_URL` at `db.rs:6` — watcher historically used `ANCHR_` variant via `health.rs` indirectly, so test with `ANCHR_DATABASE_URL` diverged from `db::resolve_db_path` path; now fixed to `ANCHOR_`).
* No shutdown (`JoinHandle` not stored), no `reload` on `PUT /sources/:id/config` (scheduler stub claims it, watcher ignores).

**Impact:** flaky `test_folder_watcher_picks_up_new_files` (now 16/16 fresh but fails in 105s full suite per `handover-2026-08-21.md:49`), hard to unit-test, leaks `notify` `Receiver` across `await` partly mitigated by `to_sync` collect but still blocks tokio thread pool.

**Fix:** extract `src/watcher/service.rs`:
```rust
pub struct WatcherService { pool: DbPool, settings: Arc<SettingsService>, jobs: Arc<JobManager>, handles: Mutex<HashMap<i64, WatchHandle>> }
impl WatcherService { fn reload(&self, sources: Vec<SourceRow>){ /* abort + spawn per-source notify task with debounce 3s + mpsc */ } }
```
Use single `DbPool` (see §4.1), `mpsc::channel` for events, `tokio::select!` for ticks. Test with `tempdir` + `FolderConnector::new` — no need for `cargo run` integration.

---

## 4. High severity — P1 (ship debt)

| # | Area | Location | Finding & consequence | Fix |
|---|------|----------|-----------------------|-----|
| 4.1 | **DB connection cost** | `db.rs:25 init_db` does pragma+`Migrations::to_latest`+`ensure_columns`+`ensure_vec`+backfill per handler (`health.rs:18`, `sources.rs:121`, `settings.rs:222`, `system.rs:41`, `pii.rs:433` each call it) | Pragma 5× + migration table scan per HTTP request → tail latency under concurrency; also `busy_timeout 5000` per open is correct but repeated | Pool: `Arc<Mutex<Connection>>` or `deadpool_sqlite`/`r2d2_sqlite`; run `Migrations::to_latest` once at boot, then `Connection::open(path)` + pragmas only. Share via `AppState { pool: DbPool }`. |
| 4.2 | **Settings cache incoherence** | `settings.rs:38 static ENV_CACHE:OnceLock<…>` dead code warning, `settings.rs:174 cached_get` does `Connection::open(&db_path)` (no WAL/FK) + reads `ANCR_DATA_DIR` not `state.data_dir`, TTL `Mutex<HashMap>` contends | `GET /settings` can see different DB than `GET /sources` when test sets `ANCHOR_DATA_DIR` + `ANCHOR_DATABASE_URL`; cache never invalidated on `PUT /settings` for other keys (only `invalidate(key)`) | Remove `OnceLock`, inject `DbPool`, use `RwLock<HashMap>` + `Instant`, open via pool. Derive typo single env read. |
| 4.3 | **Secret fields divergence** | `secrets.rs:4 SECRET_SOURCE_FIELDS=[token,api_key,password]` vs `sources.rs:14 SECRET_FIELDS+=[client_secret]` ; `secrets.rs:118 store_source_config` / `:155 resolve_source_config` dead code warnings (never called) | `client_secret` not encrypted in `secrets::` path; may persist in `sources.config` JSON plaintext. `secrets.rs:53` `&digest[..32]` truncates sha256/64 → collision 2^-128→2^-64 (still safe but needless) | Unify const `crate::secrets::SECRET_FIELDS: &[&str]= &["token","api_key","password","client_secret"]`, make `sources.rs` import it, delete dead helpers or wire them, keep 64 hex chars. |
| 4.4 | **DocType → distill never fires** | `pipeline.rs:195-213` `doc_type="general"` hardcoded; `classifier.rs:17 prompts_for(doc_type)` never called with LLM, `pipeline.rs:353 do_distill = DISTILL_DOC_TYPES.contains(&doc_type) && distill_enabled` → only `general` qualifies so distill may fire for wrong docs; test `test_distillation 5/5` passes via `classifier::distill_rules` directly | Production `distilled` chunks missing for meeting logs (B18 value) | Call `let doc_type = classifier.detect_document_type(&windows[0], cloud_trusted("classifier")).await` (reuse `cloud_trusted` gate). Add `ANCHOR_DISTILL_FORCE=1` for tests to force. |
| 4.5 | **Retrieval SQL not parameterized** | `retrieval.rs:197 sql.push_str(&format!(" AND i.source_id IN ({})", list))`, `retrieval.rs:488 who_knows_search: … format!(" OR e.summary LIKE '%{}%'", t.replace('\'',"''"))`, `graph_expand:528`, `load_hits:277` | `t` user input → injection (`who`/`owns` terms). `%_` wildcards poison LIKE. `source_id` trusted ints but pattern repeated elsewhere for `project_ids` | Use `rusqlite` params: `let ph=vec!["?"; ids.len()].join(","); sql.push_str(&format!(" AND i.source_id IN ({ph})"));` + `params_from_iter(ids)`. For LIKE, `stmt.prepare("… LIKE ?")` + `params![format!("%{}%", t)]` with `escape` . Add test `who_knows_sql_injection`. |
| 4.6 | **RRF + ranking correctness** | `retrieval.rs:54 rrf_fuse_multi`, `retrieval.rs:93 Hit::key() = if entity_id.is_none() chunk_id else -entity_id` | Collision if `chunk_id == -entity_id`; `entry.score += weight/(60+rank)` computed but also `hit.score+=add` dead; `fuse_and_rank:582` fakes scores `1/(61)` when single list → ranking vs Python `RRF_K+rank` semantics differs | Add `Hit.key()` → `(chunk_id, entity_id)` tuple hash, remove dead `hit.score` line, add property: `rrf_fuse([h1],[h1])` yields `2/(61)` not `1/(61)`. Match `backend/app/answer_engine.py:96` exactly and test vs Python snapshot. |
| 4.7 | **Per-hit N+1 queries** | `retrieval.rs:593 fuse_and_rank` `conn.query_row("SELECT created_at FROM chunks WHERE id=?1", [h.chunk_id])` inside loop + `retrieval.rs:593` `age_decay` per hit | 8 hits × 3 lists = 24 extra queries per `POST /qa`; under load p50 blow-up | Batch `SELECT id, created_at FROM chunks WHERE id IN (…)` once, join into `HashMap<i64, DateTime>`. Pass `now: DateTime<Utc>` explicitly for determinism. |
| 4.8 | **Status/heuristic drift** | `system.rs:54 ollama_reachable = !(base.contains("localhost:1"))` literal vs `health.rs:44 is_ollama_reachable` real `reqwest` probe 3s | Frontend “Ollama offline” permanently on Rust even when real Ollama up; `retrieval.avg_latency_ms 0.0` stub | Share `health::is_ollama_reachable` (inject `SettingsService` cache), add `throughput::RetrievalTracker` port (calls/vec0_calls/avg_ms). |
| 4.9 | **Panic on schema drift** | `sources.rs:123 prepare("…").unwrap()`, `entities.rs:91 conn.prepare(&sql).unwrap()`, `projects.rs:50 …unwrap()`, `system.rs:106 conn.prepare(&sql).unwrap()` with dynamic `component` | Any migration gap panics tokio task → 500 with no log, not 422 | Convert to `map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"detail":e.to_string()}))))` and `tracing::error!`. |
| 4.10 | **Jobs concurrency race** | `jobs.rs:70 pending:Mutex<VecDeque<i64>>` never used, `maybe_promote:97-117` does `SELECT COUNT(*) running` → `SELECT pending LIMIT slots` → `UPDATE running` non-atomically | Two concurrent `POST /sources/:id/sync` can promote same `pending` twice, exceeding `MAX_CONCURRENT=2` | `BEGIN IMMEDIATE;` txn or `UPDATE jobs SET status='running' WHERE id IN (SELECT id FROM jobs WHERE status='pending' ORDER BY created_at LIMIT ?1 RETURNING id)` (requires SQLite 3.35+). Add concurrent `create_job` test. |
| 4.11 | **MCP client churn** | `bin/mcp.rs:22 http_call` builds `reqwest::Client` per call, 10s timeout, empty `Bearer` header when token empty | TLS re-handshake per tool, `ANCHOR_MCP_TOKEN` leaked in error `format!("backend {method} {path} failed…")` | `OnceLock<Client>` global, reuse, mask token. |

---

## 5. Medium — P2 (design debt, not blocking)

* **Answer gate hack leaks test into prod** `answer.rs:114` `if name LIKE '%trusted%'` for `test_cloud_answer_trust_flag_allows_sensitive`. Also `answer.rs:104` inserts two `qa warning` `system_events` on *every* `POST /qa` (not just degraded path) → `GET /system/errors?component=qa` grows 2×N_qa (86s suite pollutes, expected exactly 2 in `test_qa_failure_records_warning`). Gate should read `ANCHOR_CLOUD_TRUST` via `SettingsService`, not DB name; warnings inserted only when `vector_search_vec0` falls back / LLM unreachable.
* **`expand_context` mutates `Hit.content`** `retrieval.rs:652 h.content = format!("{}\n\n[continued]\n\n{}", …)` — citation snippet grows unbounded on repeated `POST /qa/search` with same chunk. Python keeps `hit["expanded"]` separate. Store `Hit.expanded: Vec<String>` and render at answer layer only.
* **Who-knows stopwords drift:** `retrieval.rs:38`, `retrieval.rs:482` duplicate `STOPWORDS` set; `is_who_knows:25` regex recompiled per call. Hoist to `OnceLock<Regex>` + `const STOPWORDS: &[&str]`.
* **`connect_timeout` split:** `settings.rs:300` respects `ANCHOR_HTTP_CONNECT_TIMEOUT=0.2` (suite fast), `answer.rs:358 generate_answer` hardcodes 10s — `httpx.ReadTimeout 30s` in `backend/tests/conftest.py:42` masks it. Unify via `settings.get_float("http_connect_timeout",0.2)`.
* **`system.rs:104 errors_handler` 4-arm copy-paste** (38 lines duplicated). Replace with helper `fn query_events(conn, component, level, limit) -> Vec<Value>`.
* **Logging & redaction:** no `RedactingFormatter` equiv. `main.rs:151 display_str = expanded.display()` and `tracing::info!("watcher … source {} at {}", sid, display_str)` may log secret-bearing `config.path` if ever mis-stored. Wrap secrets in `SecretString` and add `tracing` filter.
* **Logs endpoint unbounded:** `system.rs:283 logs_handler` scans `data_dir` each call, no pagination/max_size — `GET /system/logs/anchorcore.log.999` can serve arbitrary file if regex bypassed. Keep 404 guard but add `Content-Length` limit.
* **Version sync missing:** `rust/Cargo.toml:6 1.0.8` claimed single source (`backend/app/config.py:11` via `scripts/sync_version.py`) but script absent per `handover-2026-08-21.md:50` R6.6 TODO. CI not checking drift.
* **Stale deps:** `rusqlite 0.31` → 0.32, `notify 6` → 8, `rand 0.8` → 0.9, `keyring 2` service API changed, `fernet 0.2` unmaintained (`age`). Not blocking but migration cost compounds.

---

## 6. What the numbers really mean

* `docs/port-review-2026-08-21.md:70` claims “ready for R6.5/6.6 polish to 121 green cutover” with `cargo check warnings only (68)`, `cargo test 37`. Actual `cargo check` now 71 warnings (`cargo check -p anchorcore` at `rust/`), 4 dead-code warns (`secrets::store_source_config`, `settings::ENV_CACHE`/`get_float`, `system::LogFileOut`). Warnings mask real dead paths (secrets helpers).
* `backend/tests` 121 vs 124 confusion: `docs/port-review-2026-08-21.md:32` says `121 tests ~25s`; handover says `124/3` fresh (was `96/30` at `faae332`). Both true — `121` is Python-only total, `124` is vs Rust with 3 `skipIf ANCHOR_TEST_RUST_URL` (`test_gdrive`, `test_pipeline_decomposition`). Not a regression, but doc mismatch to fix.
* Single-maintainer tradeoff from `docs/rust-port.md:1` §2 still holds: Rust wins packaging reliability (one binary, `render.com` vs toolcache CPython `enable_load_extension` crash `v1.0.1`) but LLM inference dominates latency (`docs/rust-port.md:28`). Port speed without `vec0` embed is not proven.

---

## 7. Security / Reliability / Observability checklist

* **Authz:** `auth.rs:1` handlers exist (`/auth/signup|login|me|status`) but **no middleware guards** mutating routes. `ANCHOR_AUTH_SECRET` enabled still allows anonymous `POST /sources` / `POST /sources/:id/sync`. Port `backend/app/auth.py: require_auth` via `axum::middleware::from_fn(require_auth)`.
* **Single writer:** SQLite `WAL` + `busy_timeout 5000` (`db.rs:31`) correct, but per-request `init_db` without `BEGIN IMMEDIATE` on job promotion risks `database is locked` flake seen in `handover-2026-08-21.md:81` (“job did not finish within 30s”). Pool with `PRAGMA synchronous=NORMAL` + `journal_size_limit` advised.
* **Logs:** `main.rs:82 ensure anchorcore.log` writes `AnchorCore Rust {version} started\n` unconditionally — races with Python’s `RotatingFileHandler`. Use `OpenOptions::append` + `tracing_subscriber::fmt::writer`.
* **Tracing:** `main.rs:71 EnvFilter::from_default_env` with no default (`RUST_LOG` unset → no logs). Set `EnvFilter::try_from_default_env().unwrap_or("info".into())`.

---

## 8. Recommended next slices (least-conflict, file-disjoint)

**Week 1 — correctness, no file overlap:**

1. `retrieval.rs:1 + answer.rs:76` — parameterize SQL, batch `created_at`, `OnceLock<Regex>`, remove `LIKE '%trusted%'`, add `Hit.key()` fix, `expand_context` separate field. Owner: reviewer A. DoD: `cargo test` + `pytest -k "graph or planner or dispute" --override-ini="addopts="` forced Rust green.
2. `pipeline.rs:1 + embedder.rs:1 + db.rs:25` — wire `embed_gated` + `sync_vec`, fix `doc_type`, extract `DbPool` (pool creation only). Owner: reviewer B. DoD: `vec_chunks` row count >0 after `POST /sources/:id/sync` with `is_pii=false` chunk.

**Week 2 — structure:**

3. `main.rs:92 + watcher/service.rs (new) + scheduler.rs:30` — extract `WatcherService`, `reload_sources` with `notify 6` → `8` if time, `JoinHandle` map, `mpsc` debounce 3s like `folder_watch_debounce`. Owner: single dev (overlaps watcher only).
4. `secrets.rs:1 + sources.rs:14 + settings.rs:1` — unify `SECRET_FIELDS`, fix trunc 32→64, remove dead `ENV_CACHE`, `LogFileOut` dead code, `cargo check` 0 warnings. Owner: reviewer A (parallel).
5. `auth.rs:1 + main.rs:240` — add `require_auth` middleware when `auth_enabled()`, add `cargo test` for 401 on `/sources` create. Owner: reviewer B.

**Cutover redefinition (`rust/docs/cutover.md:1`):**

```
- cargo check -p anchorcore: 0 warnings
- cargo test -p anchorcore: >=42 passed (37 + 5 new retrieval/embedding)
- PYTHONPATH=backend ANCHOR_TEST_RUST_URL=http://127.0.0.1:8123 pytest -q --ignore=test_mcp.py
  on fresh /tmp/ac-full-test.db : 124 passed / 3 skipped AND forced-Rust graph/planner subset green
- cargo build --release + codesign + curl /health|/system/status|/pii/config|/|/qa/search  → retrieval.backend=vec0, avg_latency_ms <20ms after 500×768d bench
- backend/app/config.py:11 version == rust/Cargo.toml:1 version (scripts/sync_version.py exists + CI check)
```

Until then **Python remains shipped artifact** (`rust/docs/cutover.md:1` gate).

---

## 9. File audit for next handover agent

Run before push:

```bash
source $HOME/.cargo/env
cargo check -p anchorcore 2>&1 | grep -c "warning:"  # must be 0
cargo test -p anchorcore   # >=42
rg "format!.*IN \(" rust/crates --no-heading  # must be 0
rg "Regex::new" rust/crates --no-heading      # only in OnceLock init
rg "init_db" rust/crates/anchorcore/src/main.rs --no-heading  # only at boot
rg "LIKE '%trusted%'" rust/crates --no-heading # must be 0
```

Checklist mirrors `docs/port-review-2026-08-21.md:128` but stricter.

---

## 10. Preservation — keep, don’t regress

* Keep `PRAGMA foreign_keys=ON` (`db.rs:35`, `db.rs:62` after migrations) — deletions rely on it (`AGENTS.md:1` gotcha 1). Keep `validate-before-write` pattern from `projects.rs:70` (validate `source_ids` before `INSERT`).
* Keep `spawn_blocking` for every `Connection` — no `Connection` across `await` (`main.rs:133` comment still true).
* Keep per-key `secrets.enc.<sha256>` (fix truncation only) — old single-blob wipe bug must not return (`AGENTS.md:1` secrets note).
* Keep `include_dir!` SPA fallback (`frontend.rs:27`) last, `CorsLayer::permissive` for dev.

---

### Appendix — Key file map (for `read` navigation)

* `rust/crates/anchorcore/src/db.rs:5` `resolve_db_path` + `:25 init_db` + `:94 ensure_vec` + `:146 migrations`
* `rust/crates/anchorcore/src/secrets.rs:1` `SecretStore` + `:113 source_secret_key`
* `rust/crates/anchorcore/src/settings.rs:94` `get/put/test-connection` + `:300 connect_timeout`
* `rust/crates/anchorcore/src/retrieval.rs:1` `RRF/age_decay/Fts/Vector/WhoKnows/Graph` + `:511 graph_expand` + `:573 fuse_and_rank`
* `rust/crates/anchorcore/src/answer.rs:76 ask` + `:114 %trusted% hack` + `:182 gater`
* `rust/crates/anchorcore/src/pipeline.rs:56 sync_source` + `:195 doc_type` + `:353 distill` + `:428 embed stub`
* `rust/crates/anchorcore/src/main.rs:70 AppState` + `:92 watcher loop` + `:240 Router`
* `rust/crates/anchorcore/src/system.rs:14 status_handler` + `:104 errors_handler` + `:283 logs_handler`
* `rust/crates/anchorcore/src/auth.rs:1` `hash_password` PBKDF2 200k + `:91 create_token` HS256

*Generated 2026-08-21 for handover. Source of truth: `rust/BACKLOG.md:1`, `rust/docs/cutover.md:1`, `docs/rust-port.md:1`, `AGENTS.md:1`, `git log 9ea85ed..9eaa9a4`.*
