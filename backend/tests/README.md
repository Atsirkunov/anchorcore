# AnchorCore — Test Suite

Maintained catalog of the backend test suite. Run: `python -m pytest tests -q`
from `backend/`. CI runs this plus the frontend build on every push
(`.github/workflows/ci.yml`).

**Conventions**
- New functionality ships with tests in the matching file (see below).
- Keep tests fast-failing and hermetic: the fixture points Ollama at
  `http://localhost:1` (unreachable), so tests exercise degradation paths
  locally. Tests that need real models are run manually as probes (see §10).
- `client` fixture = FastAPI TestClient with app lifespan (migrations run,
  scheduler starts).

---

## 1. `tests/test_smoke.py` — end-to-end flows (15 tests)

| Test | Covers |
|---|---|
| `test_health` | `/health` responds 200 |
| `test_folder_ingest_flow` | folder source → sync job → entities; hash dedup on re-sync; QA degrades gracefully |
| `test_delete_source_cascades_entities` | deleting a source removes its items + entities + chunks (NOT NULL FK + watcher unschedule fix) |
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
| `test_cancel_running_job` | long reclassify cancellable; cancelling finished job → 409; unknown → 404 |
| `test_orphaned_running_job_is_cancelled` | job left 'running' by a dead process is swept on JobManager init + via endpoint |

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

## 3. `tests/test_retrieval.py` — B12 + B12.1 retrieval quality (11 tests)

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
| `test_diversity_cap_limits_per_item` | per-item cap: one file can't monopolize results |
| `test_diversity_cap_relaxed_for_single_file` | single-file corpus isn't capped below top_k |

## 4. `tests/test_graph_retrieval.py` — B32 graph-based retrieval (5 tests)

| Test | Covers |
|---|---|
| `test_graph_expand_surfaces_connected_entities` | supersedes/depends_on/owns neighbors pulled in |
| `test_graph_expand_ranks_strong_kinds_first` | supersedes beats related in ranking |
| `test_graph_expand_excludes_stale_and_caps_fanout` | stale connected entities excluded; fan-out ≤ cap |
| `test_graph_kind_weights_are_sane` | weight ordering (supersedes > depends_on > related) |
| `test_qa_includes_graph_citations` | Q&A returns connected-entity citations |

## 5. `tests/test_planner.py` — B17 Planner→Executor→Synthesis (6 tests)

| Test | Covers |
|---|---|
| `test_planner_hybrid_always` | hybrid tool always selected |
| `test_planner_adds_who_knows_for_ownership_questions` | who/owns/responsible → who_knows tool |
| `test_who_knows_surfaces_owner_entities` | who_knows ranks owner/author entities (no embeddings needed) |
| `test_executor_runs_planned_tools` | executor returns evidence bundle; hybrid hits non-empty |
| `test_multi_tool_evidence_fusion` | who_knows + hybrid fuse into one ranked list |
| `test_qa_multi_source_cited` | DoD: decision + person evidence both cited |

## 6. `tests/test_settings.py` — B4 runtime LLM config (14 tests)

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
| (classifier/embedder provider + key storage + masked-config variants) | cloud classifier/embedder config paths |

## 7. `tests/test_classification.py` — document-aware classification (8 tests)

| Test | Covers |
|---|---|
| rule vs LLM fallback, window hashing, doc-type pre-pass, per-type prompts | B26 classification surface |

## 8. `tests/test_followups.py` — follow-up questions (5 tests)

| Test | Covers |
|---|---|
| history payload accepted; rewrite used for retrieval; no-key → raw question; pydantic turns; conversation in generation | follow-up rewrite + context pass-through |

## 9. `tests/test_projects.py` — B15 scoped search (5 tests)

| Test | Covers |
|---|---|
| `test_project_crud` | create/read/patch/delete + default project |
| `test_project_rejects_unknown_source` | unknown source id → 422 (no DB write) |
| `test_default_project_is_singleton` | setting a new default clears the old one |
| `test_project_scopes_qa_results` | DoD: overlapping projects return project-scoped citations |
| `test_project_sources_relationship` | many-to-many source membership |

## 10. `tests/test_distillation.py` — B18 distillation + IDF gating (5 tests)

| Test | Covers |
|---|---|
| `test_distillation_produces_units` | chat-like source → normalized Q&A units (`kind='distilled'`) |
| `test_distilled_unit_is_embed_min_signal` | distilled units are substantive (not gated out of embedding) |
| `test_filler_stays_unembedded_and_keyword_findable` | low-signal filler: `embedding=NULL`, still FTS-findable |
| `test_distillation_hash_skips_on_resync` | unchanged content re-syncs without duplicating units |
| `test_distilled_chunks_do_not_duplicate_on_reclassify` | reclassify drops old units before re-adding |

---

## 11. Shared helpers

- `tests/conftest.py` — temp DB + data dir env vars, unreachable Ollama,
  TestClient fixture.
- `tests/test_smoke.py::start_and_wait` / `wait_job` — poll a background job
  to completion (used by retrieval tests too).

## 12. Manual probes (not automated — need real Ollama + data)

Run with the app booted and models pulled to validate retrieval quality
against real corpora (e.g. the 237-page rulebook):

- Ask: `What are the DVCA movement rules?` → expect §4.25 DVCA sections top-ranked.
- Ask: `What are the movement rules for Merger and related movements?` → expect §4.39 MRGR + no unrelated §457 PLAC.
- Ask: `Who owns the billing migration?` → expect the planner to run `['hybrid', 'who_knows']` and cite person + decision evidence (B17/B32).
- System tab → Classifier throughput shows windows + avg latency.
- Reclassify a source after retrieval changes; probe again.

---

## 13. Adding tests (checklist)

- [ ] Pick the right file (smoke = flows, system = B1/events/redaction, retrieval = B12+, graph = B32, planner = B17, projects = B15, distillation = B18)
- [ ] Reuse `client` + `tmp_path` + `start_and_wait`; don't touch shared DB state assumptions
- [ ] If asserting exact math (RRF/decay), keep tolerance `< 1e-9` style where deterministic
- [ ] Remember the suite runs **without** Ollama — end-to-end tests must pass degraded
- [ ] Run `python -m pytest tests -q` from `backend/` before committing

---

## 14. Future logic validation tests (by backlog item)

Tests to write **when the backlog feature lands** — each validates the
invariant in the DoD, not the happy path. Add the new test file to the
tables above as they land and mark the item `— DONE`.

| Backlog item | Tests to write |
|---|---|
| **B3** Dispute tracking | dispute entity → `disputes` row written (entity_id, reason, user, timestamp); counter increments; disputed entities excluded from Q&A citations; re-verify clears counter? (per spec) |
| **B7** Source config editing | PUT updates config/name/enabled; secret fields stay keychain-backed (not echoed); invalid config rejected; watcher/scheduler reloads after edit |
| **B8** First-run wizard | wizard state transitions (no sources → guided → connected); Ollama-missing path offers instructions; sample question flow works end-to-end |
| **B9** Document type coverage | `.docx`/`.pptx`/`.odt` extract text; encrypted/corrupt file fails gracefully (recorded event, no crash); mixed-folder sync handles all types |
| **B14** MCP agent access | local stdio server: tool list exposed (`ask`, `search`, `get_entity`, `get_source`, `list_sources`, `memory_status`); `search` respects dispute/stale filters; HTTP transport: no token on remote → 401, valid token → 200, every call audited in `system_events` (component `mcp`); `ingest` (v2) creates `unverified` items authored `mcp:<token>` routed to review |
| **B16** who_knows (full) | ranking: more entities + higher confidence + recency wins; every surfaced person has cited evidence entities; no evidence → not surfaced (B17 ships a minimal tool already) |
| **B30** Data labeling / PII gating | label a source `pii` → cloud classifier/embedder/answer never touch it; shared link answers only from `public`; every gate decision audited |
| **B12 optional** LLM rerank | reranker rescoring changes top-k order per spec; candidates capped; rerank failure falls back to RRF order |
| **B2/B11/B13 regressions** | job polling survives scheduler restart (watchdog); parallel classification preserves window order (results concatenated in ref order); orphan-kill on port works on Windows/macOS |

## 15. Test-to-backlog traceability

Tests in the suite today map to shipped features:

| Backlog item | Test file (§) |
|---|---|
| B1 error transparency | test_system.py (all 8) |
| B2 jobs/progress | test_smoke.py (jobs, history, reclassify, cancel, orphan sweep, 404) |
| B4 runtime LLM config | test_settings.py (all 14) |
| B5 full-doc chunking | test_smoke.py (`test_full_document_chunking`) |
| B6 embedding backfill | test_system.py (`test_health_reports_pending_embedding_count`) |
| B11 parallel classification | manual probes only (throughput in System tab) — consider automated latency-order test |
| B12 hybrid + cleaning + heading chunking | test_retrieval.py (first 6) |
| B12.1 RRF/decay/diversity/context | test_retrieval.py (last 5) |
| B13 dev hardening | CI itself (pytest + frontend build on push) |
| B15 scoped search / projects | test_projects.py (all 5) |
| B17 planner/executor/who_knows | test_planner.py (all 6) |
| B18 distillation / IDF gating | test_distillation.py (all 5) |
| B20 packaged app | CI release.yml + build.ps1 (manual) |
| B21 sample dataset | test_smoke.py ingest flows + sample corpus |
| B23 cloud classification | test_settings.py (cloud providers) |
| B24 macOS build | CI release.yml + build.sh (manual) |
| B25 release workflow | CI release.yml (tag → artifacts) |
| B26 doc-aware classification + window context | test_classification.py (all 8) + test_smoke.py (`test_review_endpoints`) |
| B32 graph retrieval | test_graph_retrieval.py (all 5) |
| — source deletion (FK cascade + watcher fix) | test_smoke.py (`test_delete_source_cascades_entities`) |
