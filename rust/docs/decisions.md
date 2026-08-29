# Decisions — Rust Port

ADR log. Append newest first.

---

## 2026-08-29 — Phase 13: Cyclomatic complexity ≤ 15 CCN as a standing design rule

* **Decision:** Every function in `rust/crates/anchorcore/src` must stay ≤ 15 CCN (lizard, `-C 15`); `is_public_path` (`auth.rs:216`, CCN 22) is the single accepted decision table and is explicitly do-not-touch. This is now a standing design rule, not a one-off cleanup — see `rust/AGENTS.md` "Cyclomatic complexity" + root `AGENTS.md` workflow step 0.
* **Context:** A 2026-08-28 lizard sweep found 24/329 functions warned, worst `classify_and_store` CCN 73. Phase 13 (`rust/BACKLOG.md:163`) reduced all of them in 6 file-disjoint tasks: `pipeline.rs` `ClassifyCtx` phases + `with_db`/`run_db` boilerplate helper; `system.rs` `error_row` + dynamic `params_from_iter` (4 identical match arms → 1); connector parse helpers (`parse_issue`, `parse_file`, `graphql_error`); handler helper extraction across `entities.rs`/`pii.rs`/`review.rs`; `main.rs` `build_router`/`build_state` phases + watcher `fetch_and_compare`; `answer.rs` `llm_rewrite`/`heuristic_rewrite` + retrieval row converters (`hit_from_row10`, `fallback_hit_from_row8`).
* **Verification (unchanged behavior):** lizard 24 → 1 warning (the accepted one); `cargo test -p anchorcore` 49 green; `cargo check` 0; conformance `110 passed / 17 skipped` identical (CI env: `ANCHOR_DATABASE_URL=sqlite:////tmp/ac-ci-test.db`, `ANCHOR_OLLAMA_BASE_URL=http://localhost:1`, `ANCHOR_SECRETS_NO_KEYRING=1`); clippy warnings 38 → 32 (no new lints).
* **Consequences:** New handlers must be born split (guard clauses + named helpers) rather than refactored later; `lizard -C 15` joins `cargo test` + `cargo check` in every task's acceptance gate; `is_public_path` must not be touched without an explicit decision to revise the route table.

---

## 2026-08-19 — R0.2: Bundled inference decision — llama.cpp sidecar vs B8 wizard

* **Decision:** **Defer bundled llama.cpp/llamafile; ship B8 first-run wizard (Ollama guided install + model pull) as the distribution fix. Revisit bundled inference only if B8 fails to lift tester install success.**
* **Context:** `docs/rust-port.md:4` and `docs/packaging.md:78` both identify the true non-dev blocker as the Ollama prerequisite, not Python vs Rust. A bundled llama.cpp sidecar would make the app fully offline (no external runtime) but adds 2–3 days, +1–2 GB artifact, model selection/lifecycle, and a new native dependency to cross-compile and sign. B8 wizard is 0.5–1 day, keeps the current Ollama stack, and directly addresses the "can I install Ollama?" failure while preserving `rust/` incremental port for packaging reliability.
* **Alternatives considered:**
  - *A: Bundle llama.cpp now* — single offline binary, highest one-click success, but largest scope, slowest iteration, duplicates B8 work.
  - *B: B8 wizard only* — cheapest, leverages existing Ollama distribution, unblocks testers this sprint.
  - *C: Both in parallel* — rejected: doubles packaging risk during active Rust port; wizard lessons inform whether bundled inference is even needed.
* **Consequences:**
  - Immediate: keep `ollama_base_url` + `classifier_model`/`embed_model` as source of truth (`backend/app/status.py:19` `ollama_reachable`, `missing_ollama_models`); wizard probes `GET /api/tags` and offers per-platform install links + `ollama pull`.
  - Packaging stays one binary (`cargo build --release` + `include_dir! frontend/dist` `rust/crates/anchorcore/src/frontend.rs:1`), no extra sidecar to collect in `packaging.spec`.
  - Metric for revisit: if B8 wizard leaves >20% install failure or testers request offline air-gapped use, schedule llama.cpp spike (llamafile 2–3 day estimate).
  - No change to Rust port plan: `rust/BACKLOG.md:1` R1–R5 unchanged; wizard and Rust port are parallel, not competing.

---

## 2026-08-19 — R0.1: Retrieval perf spike — vec0 in Python already lands the win

* **Decision:** Keep `vec_chunks` vec0 index in Python as primary retrieval path (already `backend/alembic/versions/b33a0c1` + `backend/app/answer_engine.py:611` `vec0` with Python-scan fallback); add `RetrievalTracker` p50 reporting to `/system/status.retrieval` before Rust. Rust `retrieval.rs:1` must beat this benchmark, not replace it prematurely.
* **Context:** Hot-path profiling shows LLM latency dominates; retrieval scan was the only CPU-bound bottleneck. `backend/app/throughput.py:29` `RetrievalTracker` now records `vec0` vs `scan` + p50; `rust/scripts/bench_retrieval.py` (R0.1) benches both paths on a synthetic corpus. Rust `sqlite-vec` statically linked eliminates loadable-extension failure (`docs/rust-port.md:1` release crash) but does not yet change user-visible latency more than vec0 already does.
* **Measured (synthetic, 500 chunks, nomic-embed-text 768d, macOS M2):** `bench_retrieval.py` — vec0 p50 ~8–12ms, scan p50 ~45–90ms (10–100× on larger corpora per `docs/rust-port.md:4`). `cargo test -p anchorcore` vector tests pass both paths. Decision: do not delay wizard/port on further Python tuning; use these numbers as Rust acceptance gate.
* **Consequences:** Python stays shipped until Rust passes conformance on same SQLite file; `backend/app/db.py:36` keeps fallback when `vec0` module absent so tests `backend/tests/test_retrieval.py:1` stay green without extension.

---

## 2026-08-19 — R1.2: rusqlite_migration over refinery

* **Decision:** Use `rusqlite_migration` in `rust/crates/anchorcore/src/db.rs:1` with idempotent `migrations/*.sql` (IF NOT EXISTS + `ensure_columns`) rather than `refinery`.
* **Context:** Alembic is source of truth; Rust must apply history on both fresh and Python-created DBs without inventing schema. `rusqlite_migration` handles `PRAGMA foreign_keys=ON` + `WAL` + `migrations.to_latest` idempotently and matches `backend/app/db.py:44` pragmas.
* **Consequences:** New Rust tables must have a matching Alembic migration; Rust devs run `cargo test` against `data/anchorcore.db` not a separate file.

---
