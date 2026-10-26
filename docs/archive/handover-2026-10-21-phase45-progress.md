# Handover 2026-10-21 (evening): Phase 45 progress + all pending work

For the next agent / owner. Supersedes the *status* parts of
`docs/archive/handover-2026-10-21-llm-modes.md`. That file stays the design
reference for Phase 45 (§2 findings, §4 target design, §7.2 owner
decisions); this file covers what landed, what's left, and the rest of the
open backlog.

---

## 1. State of the tree

- `main` at `be16ccf`, **20 commits ahead of `origin/main` (incl. this handover), not pushed**.
  Last tag `v1.0.34`. No version bump or release notes for this work yet.
- Gates on `be16ccf` (macOS, `ANCHOR_SECRETS_NO_KEYRING=1`):
  - `cargo test -p anchorcore`: 534 + 104 green, golden eval 1.00 incl. `dunning-01`
  - `RUSTFLAGS="-D warnings" cargo check --all-targets`: clean
  - lizard: the 7-warning baseline, nothing new
  - `npm run build` + vitest (71/71): green
- `frontend/dist` is rebuilt, but **no release binary has been built** since
  the UI changes. Run `cargo build --release` before shipping, because the UI
  is embedded.
- Schema is at **migration 24**:

  | # | Row | Change |
  |---|---|---|
  | 21 | R45.0 | drops a Python-era vec0 index (Rust hook `drop_orphan_vec0`) |
  | 22 | R45.5 | `chunks.embed_fp` + `embed_state` |
  | 23 | R45.8 | entity record columns + owner index |
  | 24 | R45.13 | `is_glossary` + vocabulary purge (Rust step) |

- Verified on a **copy** of the owner's live DB (`~/.anchorcore/anchorcore.db`,
  previously v20). It migrates to v24 and boots without errors. All 194 stored
  vectors get stamped, and vocabulary goes from 62 rows (12 LLM proposed + 50
  rule) to 3 clean rows (`sla`, `msu`, `bah`). The live DB itself was not
  touched: it upgrades on the owner's next run of a new build.
- Leftovers: the fork worktrees under `.claude/worktrees/agent-*` (6 dirs,
  branches `worktree-agent-*`) are all merged and can be deleted with
  `git worktree remove`. The R45.3 candle probe lives only in a session
  scratchpad; rebuild it from `docs/probes/2026-10-21-embedding-engine.md`.

## 2. Phase 45: what landed

| Row | Commit(s) | What it does |
|---|---|---|
| R45.0 | `47340e2` | Migration 21 removes the orphaned vec0 table, its shadow tables and triggers. Every chunk write on upgraded DBs used to fail with `no such module: vec0`. |
| R45.1 | `0c5ea6c` | `extract_entities` (`rules` default / `off` / `llm`), `distill_enabled` default false. Readiness only asks for the classifier model when `chat_model_needed`. Default ingest makes 0 chat calls (counting-mock test). |
| R45.2 | `7284eab`, merge `50a6839` | `llm_mode` (`local` default / `agent`). Agent mode returns `mode: agent_compose` on MCP `ask`, `/qa` and `/qa/public` with no chat calls; `/qa/search` is unchanged. Ask tab shows an evidence view, Settings has a selector. The answer model is dropped from readiness in agent mode. |
| R45.3 | `a6da878` | **Partial.** `docs/probes/2026-10-21-embedding-engine.md`: parity ≥ 0.99995 vs Ollama on every backend incl. f16 and Linux aarch64; Metal f16 fastest (50.6 chunks/s, 7.0 ms/query); Accelerate can't do f16; Linux CPU 2.4–3.5 chunks/s on 820-char chunks; 512-d Matryoshka lossless on the small eval. |
| R45.5 | `867fb15`, merge `a1ab60e`, `7600101` | Per-chunk `embed_fp` (`provider:model:dim`), query-side `embed_state`, model families in `embed_fp.rs`. On mismatch the stale vectors go keyword-only and the backfill is woken. `/system/status.embedder` shows `active_fingerprint` + `stale_vectors`. |
| R45.8 | `a0da30c`, `21bf107` | `records.rs` deterministic parser replaces the sentence-regex rules (record bucket). Entity columns `record_id`, `record_status`, `due`, `done`, `supersedes`, `superseded_by`; backup round-trips them. **No auto-verify**: extraction always stores `unverified`. |
| R45.9 | `ffbc5ba`, `ba4631d` | `GET /decisions` + `GET /actions` (`records_api.rs`) and the MCP `decisions` / `actions` tools. Gating mirrors `/qa/search`. Entities tab opens on Decisions + Actions views. `who_knows_search` / `graph_expand` are deleted. Phase 31 is closed into this row. |
| R45.13 | `e211d9a`, `be16ccf` | Vocabulary without proposals: Tier-2 LLM terms, the quarantine and the "Words to confirm" queue are deleted. One initialism-checked parenthetical rule. A people-owned glossary: `/glossary` CRUD plus `PATCH /items/:id {"is_glossary": true}` file parsing. Local-mode acronym expansion lives in `query_expand.rs`. R43.30 is closed into this row; the R40.7 reversal is in `decisions.md`. |

## 3. Phase 45: pending

### 3.1 Owner decisions (block work)

| # | Question | Evidence / proposal | Blocks |
|---|---|---|---|
| 7.2.1 | Embedding model + size | Probe supports **nomic v1.5 stored f16 (~274 MB)**: parity holds and no re-embed is needed. Pinned SHA-256s are in the probe doc. Alternative: a smaller model, which means a re-embed and weaker search. | R45.4 → R45.6 → R45.7 |
| 7.2.2 | Default embedder on Windows/Linux | Proposed: built-in everywhere; when there's no Metal, the first index is large (>~2k chunks) and Ollama is reachable, suggest Ollama (identical vectors). A 10k-chunk first index takes ~55 min on Linux CPU vs ~4.5 min with Ollama on Metal. Ollama's own CPU-only speed is unmeasured. | R45.4 defaults |
| 7.2.3 | Legacy entities | The owner DB has **1,798 rule-made rows marked `verified`** (pre-R45.8). They now show up in `/actions` and `/decisions` as junk, e.g. whole meeting headers stored as "actions". Proposed: (a) demote to `unverified` + (b) delete rule rows from non-record items, after a backup. | R45.10 |
| 7.2.5 | Agentic data boundary | Unchanged on purpose. Agent-mode `ask`, `/decisions` and `/actions` all mirror `/qa/search` gating (trust from `answer_base_url`). Changing it means changing all of them together (AGENTS.md notes this). | none |
| new | Local-mode acronym expansion is **on by default** (R45.13) | Each new set of unknown acronyms costs one answer-model call (≤10 s, cached in memory) before search runs. `acronym_expansion=false` turns it off. Keep on, or make it opt-in? | none |
| new | Migration 24 dropped all LLM vocabulary rows | Correct non-initialism pairs (e.g. `rfnd → refund`) must now be added to the glossary by hand. The owner DB had no approved LLM rows. | none |

### 3.2 Open rows

| Row | Pri | State / next step |
|---|---|---|
| R45.3 (rest) | P1 | **CI-only legs:** Windows MinGW build, x86 `mkl`, and Ollama CPU-only baseline. Needs a CI run; ask the owner before pushing a branch that triggers it. Then write the engine decision entry in `rust/docs/decisions.md`. |
| R45.4 | P1 | Ship the built-in embedder. Needs 7.2.1 + 7.2.2. |
| R45.6 | P1 | Wizard rework (mode-first, MCP-or-skill, RAM-sized local picker). Needs R45.4. |
| R45.7 | P2 | "How AnchorCore works" card + `docs/how-it-works.md` + website copy (handover-llm-modes §4.5). Needs R45.6. |
| R45.10 | P2 | Legacy entity cleanup migration. Needs 7.2.3; backup first; record counts before/after. |
| R45.11 | P3 | Opt-in LLM enrichment for unstructured records (`extract_entities=llm`, always `unverified`, fills `owner`). |
| R45.2b | P3 | MCP sampling when the client advertises it. |
| R45.12 | P2 | Doc drift, ongoing. Skills changed (`anchorcore-search`, `anchorcore-setup`) → bump `skills-lock.json` with the release. |

R45.4 implementation notes:
- Per-OS candle features: `metal` + `accelerate` as macOS-only target deps.
- **Metal f16 needs the attention mask as U8.** Otherwise it fails with `where_cond U32 F16 not implemented`.
- Sort texts by length before batching (Metal f32 went from 29 to 44 chunks/s).
- Download to `{data_dir}/models/` from the pinned HF revision, SHA-256 verified, resumable, plus an offline-drop path.
- Prefetch the weights in the Team image.
- The new provider tag is `builtin`. It already shares a family with Ollama nomic in `embed_fp.rs`.

### 3.3 Follow-ups surfaced while landing rows

- **R45.5:**
  - Saving `embed_model` doesn't wake the backfill; the next query does.
  - Pre-R45.5 archives restore unstamped, so their vectors are keyword-only until re-embedded.
  - An `ANCHOR_EMBED_MODEL` set only in the env is invisible to migration 22's SQL, which costs one re-embed after upgrade.
- **R45.8/R45.9:** parsing runs per 16k classify window, so a record longer than one window loses heading context in later windows.
- **R45.9:** the Glossary panel lives inside "All entities". It may deserve its own place if that view is reorganised.
- **`/system/status.retrieval`** still reports a hard-coded `backend: "vec0"` with zero counters, while AGENTS.md says it reports backend + latency. Check this against the R43.9 work.

## 4. Release prep (not started)

The next release will be 1.0.35, or 1.1.0 given new APIs and migrations. Follow `docs/releasing.md`:
- Bump `rust/Cargo.toml` (`scripts/sync_version.py`).
- Write release notes. User-facing: no chat model needed at ingest, agent mode, Decisions/Actions, glossary replaces "Words to confirm", vocabulary purge, the vec0 fix that unblocks indexing on upgraded DBs.
- Rebuild the release binary.
- Run the include audit (`grep -rn "include_\(str\|dir\|bytes\)!"`): new migration files are under `rust/` (fine for the Docker builder).
- Run the conformance suite: shared API behaviour changed (`/qa` modes, new routes, deleted `/review/vocabulary`).
- Bump skills.

## 5. Rest of the open backlog (outside Phase 45)

Row-level truth is `rust/BACKLOG.md`; this is the map.

| Phase | Open rows | Notes |
|---|---|---|
| 30 Ingestion throughput | **R30.1 (P0)** one job per source; **R30.3 (P0)** watcher fallback re-extracting every second; **R30.6 (P0)** Ollama context bound; R30.2, R30.4, R30.5 (P1) | Live P0 track. R30.6 matters less now that ingest makes no chat calls by default. |
| 43 Audit S2 (reliability/scale) | R43.9 slim scan (P1), R43.10 link sqlite-vec (P2), R43.11 job panic guard (P1), R43.12 tx + embed reuse (P1), R43.13 connector cursors (P2), **R43.14 Drive OAuth refresh (P1, Drive sync breaks after ~1 h)**, R43.15 import body limit (P2), R43.16/.17 hygiene (P3), R43.27 eval margin (P2) | R43.9 now has to respect the R45.5 `embed_fp` filter in the scan. |
| 43 Audit S3 (before any networked deploy) | R43.18 close signup + roles, R43.19 SPA login, R43.23 connector input + egress (P1); R43.20, R43.21, R43.22, R43.24, R43.26 (P2); R43.25 cargo audit in CI (P3) | Gate for Team / networked use. |
| 36 Parent-child retrieval | **R36.2 (P0)** config rule-level sections; R36.4, R36.5, R36.8, R36.15 (P1); R36.9, R36.16, R36.18b (P2); R36.17 + deferred R36.11–R36.14 (P3) | |
| 35 PII | R35.2 enforce manual marks on cloud flows (P1), R35.3 drop `chunks.is_pii` (P2) | |
| 33 Code indexing | R33.7 dispute-exclusion tests under leaf-pool retrieval (P1) | Touches the archived Python suite. |
| 20 MCP zero-setup | R20.4 `runtime.json` discovery (P1), R20.10 MCP smoke in CI (P2) | |
| 41 macOS install | R41.2 DMG (P1), R41.1, R41.3, R41.5 (P2), R41.4 notarization (P3, owner call) | |
| 39 Retrieval weighting | R39.13 (P3), R39.11 reranker (deferred) | |
| 42 | R42.1 DB size in status (P3) | |
| 44 Website | owner action: R44.20 (toggle Cloudflare WebMCP off) | |
| 18 | R18.13 social proof (partial, owner-deferred) | |

## 6. Resume here

1. Owner: answer §3.1 (7.2.1–7.2.3 + the two new questions).
2. Push `main` once reviewed (20 local commits), or cut the release in §4.
3. Then, depending on the answers: R45.10 (small, removes the visible junk),
   R45.4 → R45.6 → R45.7. In parallel, the P0s from Phase 30 (R30.1, R30.3)
   and R36.2.
4. Delete the merged worktrees (§1).
