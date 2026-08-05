# AnchorCore — Test Suite

Maintained catalog of the backend test suite. Run: `python -m pytest tests -q`
from `backend/`. CI runs this plus the frontend build on every push
(`.github/workflows/ci.yml`).

**Conventions**
- New functionality ships with tests in the matching file (see below).
- Keep tests fast-failing and hermetic: the fixture points Ollama at
  `http://localhost:1` (unreachable), so tests exercise degradation paths
  locally. Tests that need real models are run manually as probes (see §5).
- `client` fixture = FastAPI TestClient with app lifespan (migrations run,
  scheduler starts).

---

## 1. `tests/test_smoke.py` — end-to-end flows (13 tests)

| Test | Covers |
|---|---|
| `test_health` | `/health` responds 200 |
| `test_folder_ingest_flow` | folder source → sync job → entities; hash dedup on re-sync; QA degrades gracefully |
| `test_job_history_and_progress` | background job: total/processed counts, history listing |
| `test_reclassify_runs_as_job` | reclassify as async job, entities rebuilt |
| `test_review_endpoints` | low-confidence list, PATCH verify |
| `test_sync_error_tracking` | failed sync → job failure + source error_count/last_error |
| `test_job_404` | unknown job id → 404 |
| `test_health_reports_components` | health component keys (ollama, tasks, failing_sources, classifier) |
| `test_folder_watcher_picks_up_new_files` | watchdog picks up new file within 20s |
| `test_reclassify_rebuilds_entities` | reclassify keeps entity surface |
| `test_merge_cleans_references` | merge with relationship/merge rows doesn't 500 (NOT NULL FK bug) |
| `test_full_document_chunking` | long doc → ≥2 full-doc chunks; tail content chunked; QA returns citations |

## 2. `tests/test_system.py` — B1 error transparency + redaction (8 tests)

| Test | Covers |
|---|---|
| `test_redact_masks_secrets` | `redact()` masks known secret patterns |
| `test_redacting_formatter_masks_log_lines` | RedactingFormatter scrubs secrets from log output |
| `test_system_status` | `/system/status` shape (version, ollama, answer, tasks, classifier) |
| `test_sync_failure_records_structured_event` | sync failure → `system_events` row with source link |
| `test_qa_failure_records_warning` | QA degradation → warning event |
| `test_get_config_redacts_secrets` | source config endpoint masks tokens |
| `test_logs_list_and_download` | log listing + sanitized download, 404 on bad name |
| `test_health_reports_pending_embedding_count` | pending-embedding counter in `/health` |

## 3. `tests/test_retrieval.py` — B12 + B12.1 retrieval quality (10 tests)

| Test | Covers |
|---|---|
| `test_clean_text_removes_page_numbers_and_control_chars` | cleaning: page-number lines, control chars, soft hyphens, whitespace |
| `test_repeated_lines_detected_and_stripped` | cleaning: repeated header/footer detection + removal |
| `test_chunk_document_respects_headings` | heading-aware chunking: `§434`/`§437` hard boundaries |
| `test_chunk_document_splits_oversized_sections` | oversized sections split ≤ chunk_max_chars; tail content retained |
| `test_hybrid_keyword_retrieval` | end-to-end: acronym (DVCA) retrieves cited chunks without embeddings |
| `test_fts_table_exists` | `chunks_fts` virtual table exists after migration |
| `test_rrf_consensus_beats_single_vote` | RRF: chunk in both lists outranks single-list #1; exact 1/(60+rank) math |
| `test_rrf_keyword_weight_scales_list` | RRF: keyword list weight scales contribution |
| `test_age_decay_favors_recent` | age decay: 0.5^age/halflife math, recent > old |
| `test_diversity_cap_limits_per_source` | per-source cap: one source can't monopolize results |

## 4. `tests/test_settings.py` — B4 runtime LLM config (8 tests)

| Test | Covers |
|---|---|
| `test_settings_get_returns_env_defaults` | GET shape; secret key masked |
| `test_settings_put_updates_and_persists` | PUT persists across requests (DB-backed) |
| `test_settings_unknown_key_rejected` | unknown keys → 422 |
| `test_settings_secret_masked_and_stored_in_keystore` | secret → `***set***` in API, plaintext only in SecretStore |
| `test_settings_clear_restores_env` | clearing an override falls back to env default |
| `test_test_connection_ollama_unreachable` | Ollama down → graceful failure result |
| `test_test_connection_answer_requires_key` | answer provider validation path |
| `test_system_status_reflects_runtime_settings` | `/system/status` reads live (runtime) values |

---

## 5. Shared helpers

- `tests/conftest.py` — temp DB + data dir env vars, unreachable Ollama,
  TestClient fixture.
- `tests/test_smoke.py::start_and_wait` / `wait_job` — poll a background job
  to completion (used by retrieval tests too).

## 6. Manual probes (not automated — need real Ollama + data)Run with the app booted and models pulled to validate retrieval quality
against real corpora (e.g. the 237-page rulebook):

- Ask: `What are the DVCA movement rules?` → expect §4.25 DVCA sections top-ranked.
- Ask: `What are the movement rules for Merger and related movements?` → expect §4.39 MRGR + no unrelated §457 PLAC.
- System tab → Classifier throughput shows windows + avg latency.
- Reclassify a source after retrieval changes; probe again.

---

## 6. Adding tests (checklist)

- [ ] Pick the right file (smoke = flows, system = B1/events/redaction, retrieval = B12+)
- [ ] Reuse `client` + `tmp_path` + `start_and_wait`; don't touch shared DB state assumptions
- [ ] If asserting exact math (RRF/decay), keep tolerance `< 1e-9` style where deterministic
- [ ] Remember the suite runs **without** Ollama — end-to-end tests must pass degraded
- [ ] Run `python -m pytest tests -q` from `backend/` before committing

---

## 7. Future logic validation tests (by backlog item)

Tests to write **when the backlog feature lands** — each validates the
invariant in the DoD, not the happy path. Add the new test file to §1–§3
tables above as they land and mark the item `— DONE`.

| Backlog item | Tests to write |
|---|---|
| **B3** Dispute tracking | dispute entity → `disputes` row written (entity_id, reason, user, timestamp); counter increments; disputed entities excluded from Q&A citations; re-verify clears counter? (per spec) |
| **B4** LLM config in-app | settings write persists to `app_settings` and overrides env; secret fields never returned by GET (masked); "test connection" success + failure paths; runtime change takes effect without restart (no stale singleton) |
| **B7** Source config editing | PUT updates config/name/enabled; secret fields stay keychain-backed (not echoed); invalid config rejected; watcher/scheduler reloads after edit |
| **B8** First-run wizard | wizard state transitions (no sources → guided → connected); Ollama-missing path offers instructions; sample question flow works end-to-end |
| **B9** Document type coverage | `.docx`/`.pptx`/`.odt` extract text; encrypted/corrupt file fails gracefully (recorded event, no crash); mixed-folder sync handles all types |
| **B14** MCP agent access | local stdio server: tool list exposed (`ask`, `search`, `get_entity`, `get_source`, `list_sources`, `memory_status`); `search` respects dispute/stale filters; HTTP transport: no token on remote → 401, valid token → 200, every call audited in `system_events` (component `mcp`); `ingest` (v2) creates `unverified` items authored `mcp:<token>` routed to review |
| **B15** Projects / scoped search | project = bundle of sources; same source in 2 projects (no duplication); default project scopes queries (Q&A + MCP search); out-of-project source never returned |
| **B16** who_knows | ranking: more entities + higher confidence + recency wins; every surfaced person has cited evidence entities; no evidence → not surfaced |
| **B17** Planner→Executor→Synthesis | planner selects tools per query; executor fans out in parallel and normalizes evidence; synthesis cites both source types for cross-source questions; search-only path (no planner) still answers |
| **B18** Distillation / IDF gating | distilled units (question/summary/resolution) findable; filler messages absent from vector results (below IDF threshold) but present in FTS; full-doc chunks still embedded (B12 DoD holds) |
| **B12 optional** LLM rerank | reranker rescoring changes top-k order per spec; candidates capped; rerank failure falls back to RRF order |
| **B2/B11/B13 regressions** | job polling survives scheduler restart (watchdog); parallel classification preserves window order (results concatenated in ref order); orphan-kill on port works on Windows/macOS |

## 8. Test-to-backlog traceability

Tests in the suite today map to shipped features:

| Backlog item | Test file (§) |
|---|---|
| B1 error transparency | test_system.py (all 8) |
| B2 jobs/progress | test_smoke.py (jobs, history, reclassify, 404) |
| B4 runtime LLM config | test_settings.py (all 8) |
| B5 full-doc chunking | test_smoke.py (`test_full_document_chunking`) |
| B6 embedding backfill | test_system.py (`test_health_reports_pending_embedding_count`) |
| B11 parallel classification | manual probes only (throughput in System tab) — consider automated latency-order test |
| B12 hybrid + cleaning + heading chunking | test_retrieval.py (first 6) |
| B12.1 RRF/decay/diversity/context | test_retrieval.py (last 4) |
| B13 dev hardening | CI itself (pytest + frontend build on push) |
