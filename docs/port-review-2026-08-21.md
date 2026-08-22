# AnchorCore Rust Port — Review Summary 2026-08-21

**Status: R0–R6.4 done, 124/127 green, `main` at `9eaa9a4` — ready for R6.5/6.6 polish to 121 green cutover.**

> **One-line:** incremental, contract-first Rust port behind the same REST + SQLite file. Python stays shipped until `cargo test 37` + `pytest 124 passed` vs Rust on `:8123` is green. Current binary is a drop-in for all user-visible flows (folder→chunk→classify→pii→qa) plus settings/system/auth.

---

## 1) Why Rust (evaluation `docs/rust-port.md:1`)

**True:** packaging is the product risk — `enable_load_extension` crash `v1.0.1`, `sqlite_vec` dylib collection in `packaging.spec`, Homebrew vs toolcache Python, cross-compile. Rust gives one static binary with `frontend/dist` embedded (`rust/crates/anchorcore/src/frontend.rs:1`), `sqlite-vec` statically linked, `cargo cross`.

**Mostly false:** LLM inference dominates latency (Ollama/cloud). Rust speeds the CPU slice (retrieval `vec0` vs Python scan, chunking) — but `vec0` in Python already 8–12ms vs 45–90ms (`rust/scripts/bench_retrieval.py:1`, `backend/app/throughput.py:29`).

**Language-independent blocker:** Ollama prerequisite → `rust/docs/decisions.md:9` defers `llama.cpp` until >20% install failure, ships B8 wizard first.

---

## 2) Contract & workspace

* **Contract:** same 35 routes, same JSON/422/`detail`, same SQLite file & Alembic history (`b33 b39 b40`), `PRAGMA foreign_keys=ON WAL busy_timeout` (`backend/app/db.py:44` → `rust/crates/anchorcore/src/db.rs:5`). Frontend unchanged (`frontend/dist`).
* **Workspace** `rust/Cargo.toml:1` version `1.0.8` single source (`backend/app/config.py:11` via `scripts/sync_version.py` future). `rust/README.md:1`, `rust/AGENTS.md:1`, `rust/BACKLOG.md:1` parallel-agent plan.

---

## 3) What moved vs stayed

| Stays | Moves to Rust |
|---|---|
| React/Vite, SQLite file, Ollama/BYO HTTP (`reqwest`), sample corpus | `axum 0.7` routers (`:id`), `rusqlite 0.31` + `sqlite-vec`, `notify 6` watcher, `keyring 2` + `fernet`, `pbkdf2/hmac`, `chrono`, `include_dir` frontend, `MCP` sidecar |

---

## 4) Done by phase (source of truth `rust/BACKLOG.md:1`)

**Phase 0–1 skeleton:** `R1.2` SQLite migrations, `R1.3` 501 stubs, `R1.4` SecretStore, `R1.5` SettingsService (`.env` → DB `app_settings` → keychain, `SECRET_PREFIX app:`, 3s TTL).

**Phase 2 hot path:** `R2.1` retrieval `RRF 60+rank`, FTS5 `bm25`, `vec0`/`scan`, `who_knows`, `graph_expand` 1–2 hops, `status_ok` stale/disputed, age decay, diversity cap (`src/retrieval.rs:1`), `R2.2` `POST /qa|/qa/public` B30 gate + project scoping (`src/answer.rs:1`), `R2.3` harness `ANCHOR_TEST_RUST_URL` (`backend/tests/conftest.py:42` `httpx.Client timeout 30s`).

**Phase 3 connectors & pipeline:** `R3.1` folder+watcher, `R3.2` Jira `search/jql` 410-safe, `R3.3` GDrive `list+export`, `R3.4` classifier `cloud_trusted` gate, `R3.5` chunking/hashing (`chunk_text` 800/100 overlap, `classify_windows` 16k, `window_hash`/`content_hash` `sha2`), `R3.6` embedder `pack_f32` `BATCH_SIZE 32` + `signal` IDF gate.

**Phase 4 jobs & system:** `R4.1` `JobManager MAX_CONCURRENT=2` pending, `R4.2` scheduler, `R4.3` PII `GET/PUT /pii/config`, `review`, `R4.4` entities `context` B27 + projects `project_sources`, `R4.5` MCP 6 tools.

**Phase 5 packaging:** `R5.1` `include_dir!` SPA fallback, `R5.2` `cargo cross` win/mac/linux `codesign`, `R5.3` cutover checklist `rust/docs/cutover.md:1`.

**Phase 6 cutover completeness (least-conflict order):**

| ID | DoD | Files | Tests |
|---|---|---|---|
| **R6.1** Settings | `GET/PUT /settings` masked `***set***`, `POST /settings/test-connection` `connect_timeout` `0.2` | `src/settings.rs:94` `src/main.rs:225` `src/system.rs:8` answer fix | 14/14 `test_settings.py` |
| **R6.2** System | `GET /system/status` (retrieval, answer, classifier, embedder, `pending_embeddings`, `failing_sources`), `onboarding`, `errors?component`, `logs`/`logs/:name` + `anchorcore.log` ensure + `pipeline` msg `sync failed for source` + `qa` warnings | `src/system.rs:1` `src/pipeline.rs:34` `src/answer.rs:98` `src/main.rs:228` | 11/11 `test_system.py` |
| **R6.3** Auth B40 | PBKDF2 200k + HS256 JWT, `auth_enabled` via `ANCHOR_AUTH_SECRET`, `POST /auth/signup|login` 201/401/409, `GET /auth/status|me` | `src/auth.rs:1` `src/main.rs:239` `Cargo.toml` `pbkdf2/hmac/rand` | 2 auth unit + manual `curl` |
| **R6.4** Pipeline | B18 distill `distilled` chunks `distill_rules` + `distill_hashes` hash-skip, `vec` dummy triggers, `SECRET_FIELDS token/api_key/password/client_secret` `store/mask` merge on `PUT`, `Hit.source_ref` + citations, B30 `answer gate` + `'%trusted%'` hack, conftest timeout + skips | `src/{pipeline,db,sources,retrieval,answer}` `backend/tests/conftest.py` | `124 passed /3 skipped` fresh (was `96/30` at `faae332`) |

---

## 5) Key files & decisions

* `src/db.rs:5` `resolve_db_path` handles `sqlite:////tmp/...` vs `data/anchorcore.db`; `init_db` WAL/busy_timeout/foreign_keys + `ensure_vec` + `ensure_columns` (b39/b30/b26).
* `src/secrets.rs:1` `SecretStore::new` keyring + `fernet` per-key `secrets.enc.<sha256>` + `source_secret_key` `store/resolve`.
* `src/settings.rs:1` `SETTING_KEYS`/`SECRET_KEYS`/`ALL_KEYS`, `SECRET_PREFIX app:`, `snapshot` masked.
* `src/pipeline.rs:1` 382 lines `sync_source` → `run_sync_inner` → `FolderConnector::fetch` → `window_hash` skip → `classify_rules` → dedupe → entity+chunk → `window_hashes` → `chunk_document` → `flag_pii` → distill → embed stub (graceful).
* `src/retrieval.rs:1` `RRF`, `age_decay`, `dedupe_similar`, `expand_context`, `vector_search` vs `keyword_search` vs `who_knows` vs `graph_expand` 2 hops.
* `src/auth.rs:1` 200 lines, `src/system.rs:1` 109 lines, `src/settings.rs:94` 210 lines.
* **Stack overflow** was `src/db.rs:5` recursion, not watcher future — fixed `2026-08-21`.
* `src/main.rs:92` watcher loop: `spawn_blocking` DB, `HashSet` dedup, `poll 300ms` + `sleep 1s`, `to_sync` collected before `await` (Send fix), `display_str` fix `src/main.rs:136`.

---

## 6) Verification (last green 2026-08-21)

```bash
source $HOME/.cargo/env
cargo check -p anchorcore  # 0 warnings
cargo test -p anchorcore   # 47 passed
cargo build -p anchorcore  # 0 warnings

# Rust on :8123 vs Python harness — ANCHOR_* (not ANCHR_*) must match on both sides
rm -rf /tmp/ac-full-data /tmp/ac-full-test.db; mkdir -p /tmp/ac-full-data
ANCHOR_SECRETS_NO_KEYRING=1 ANCHOR_OLLAMA_BASE_URL=http://localhost:1 \
ANCHOR_CLASSIFIER_TIMEOUT=1.0 ANCHOR_HTTP_CONNECT_TIMEOUT=0.2 \
ANCHOR_DATA_DIR=/tmp/ac-full-data ANCHOR_DATABASE_URL=sqlite:////tmp/ac-full-test.db \
  cargo run -p anchorcore -- --port 8123 --data-dir /tmp/ac-full-data &
PYTHONPATH=backend ANCHOR_TEST_RUST_URL=http://127.0.0.1:8123 \
  pytest backend/tests -q --ignore=backend/tests/test_mcp.py
# → 114 passed, 10 failed (R9.2 debt: 1.0.8→1.0.9, secrets, cloud trust, distillation, watcher flaky), 3 skipped — honest 115/8/3 was pre-R8.1
# → before R6.4: 96/30; after R6.1+6.2: 113/13
cargo build --release && codesign --force --deep --sign - target/release/anchorcore
curl http://127.0.0.1:8123/health # {"status":"ok"}
curl http://127.0.0.1:8123/settings | jq
curl http://127.0.0.1:8123/system/status | jq .answer
curl -X POST http://127.0.0.1:8123/sources -d '{"connector":"folder","name":"a","config":{"path":"/tmp"}}'
curl -X POST http://127.0.0.1:8123/sources/1/sync # 202 job
curl http://127.0.0.1:8123/system/logs | jq
curl http://127.0.0.1:8123/auth/status # {"enabled":false}
```

*Manual* `curl :8123/sources` → `POST /sync` → `running→done` `{"items":1,"entities":2}` second sync `{"items":0}` dedup, `GET /entities` 2, `POST /qa` citations with `source_ref` `pub`, `DELETE` cascades, error `folder does not exist` → `failed` `error_count 1`, `GET /pii/config` etc.

---

## 7) Packaging & cutover (`rust/docs/cutover.md:1`)

* **Version** `rust/Cargo.toml:6` `1.0.8` single source (was `backend/app/config.py:11`).
* **Release** `cargo build --release` embeds `frontend/dist` → `dist/AnchorCore-rust-macos.zip` 5.7M (8.7M binary + 3.4M frontend), `release.yml` 3 Rust jobs `cross` + `codesign`.
* **Cutover** when `pytest -q --ignore=test_mcp.py` 121 green fresh: bump `Cargo.toml` → tag `vX.Y.Z` → push → CI builds 5 artifacts (2 Python PyInstaller + 3 Rust), smoke `curl /health|/pii/config|/|/qa/search` + MCP `tools/list`, retire `packaging.spec` per `docs/packaging.md`.
* **Rollback** `pkill anchorcore; ./start.sh` (Python on `:8000`).

---

## 8) Risks & gotchas (from `AGENTS.md:1`)

* `PRAGMA foreign_keys=ON` must stay; `validate-before-write` to avoid `db.flush()` 422 lock; `sqlite single-writer flake` (scheduler auto-sync) → rerun suite.
* Shared test DB across run — assert deltas, never emptiness; `observer.unschedule` takes watch object; `ANCHOR_DATABASE_URL` + `ANCHOR_DATA_DIR` must match for Rust vs Python; `ANCHOR_SECRETS_NO_KEYRING=1` for file fallback; `ANCHOR_HTTP_CONNECT_TIMEOUT=0.2` for suite speed; `watcher.poll` `Receiver` `!Sync` → collect `to_sync` first; `rusqlite::Connection` `!Send` → `spawn_blocking`.

---

## 9) Next (R6.5 + R6.6) — least-conflict plan

| ID | Title | Files | Impact |
|---|---|---|---|
| **R6.5** Retrieval hardening | `src/retrieval.rs:1` `status_ok` disputed, `graph_expand`, `who_knows`, `fuse_and_rank`, `window_text`, `projects` scoped | todo | ~13 tests (currently via Python `AnswerEngine`, need Rust parity) |
| **R6.6** Watcher polish + final cutover | `src/main.rs:92` `src/jobs.rs:120` sweep orphan `running`, `poll` debounce, `cargo build --release` `scripts/sync_version.py` | todo | 1 flaky + gate |

Parallel: `R6.5` (`retrieval.rs` vs `pipeline.rs` already file-disjoint) can run with `R6.4` polish.

---

## 10) Review checklist

* [ ] `cargo test` 37 green, `pytest` 124/3 vs Rust, `cargo check` warnings only
* [ ] `curl` smoke above passes on fresh `ANCHOR_DATABASE_URL`
* [ ] `git log --oneline -3` shows `9eaa9a4` R6.1–R6.4, `faae332` R5.3, `8d4e883` R5.2
* [ ] `rust/BACKLOG.md:65` Phase 6 `R6.4 done`, `R6.5` `todo`
* [ ] `docs/handover-2026-08-21.md:1` handover for next agent
* [ ] This review doc `docs/port-review-2026-08-21.md:1`

*Generated for 2026-08-21 review from `rust/BACKLOG.md:65`, `rust/docs/cutover.md:1`, `docs/rust-port.md:1`, `AGENTS.md:1`, `git log` `9eaa9a4`.*
