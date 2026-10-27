# Handover 2026-10-27 — two parallel batches after 1.5.0, owner decisions pending

Read root `AGENTS.md` first (gates, gotchas, owner decisions). This page holds only the state after the two batches
run on 2026-10-26/27 and what is waiting on the owner. Row text in `rust/BACKLOG.md` stays the source of truth (each
row carries its evidence); decision entries for every choice are in `rust/docs/decisions.md`.

## State of `main` at handover

- Pushed to `origin/main`, **not tagged** (last tag `v1.5.0`). `6c6d609` + this handover. The next tag (1.6.0, bump per
  `docs/releasing.md`) ships everything below.
- Batch 1 (`8c95f93`): CI green on all 7 jobs, including both MCP smoke steps (R20.5 handshake, R20.10 end-to-end search).
- Batch 2 (`6c6d609`): gates green on the merged tree locally — all 16 `cargo test -p anchorcore` targets, `RUSTFLAGS="-D
  warnings" cargo check` clean, lizard = the 7-warning baseline, `npm run build` + lint clean, vitest 99/99 on Node 26,
  `scripts/ci_mcp_search_smoke.sh` passes. CI on `6c6d609` green on all 7 jobs (run 40475170965).
- All agent worktrees (`.claude/worktrees/*`) and task branches were removed (local only, never pushed). The
  `.muse/worktrees/*` and `r40-work` worktrees predate these sessions — not touched.

## Shipped (don't redo)

| Row | What landed | Known gaps |
|---|---|---|
| R45.3 / R45.4 | Backlog only: marked shipped in 1.5.0 (CI + Release green on `v1.5.0`) | — |
| R36.2 (P0) | `config_chunks.rs`: section per top-level key / TOML table, one section per entry when a top-level list is "the main content"; identifying scalars promoted into the breadcrumb (≤32 chars); 500–800-char chunks with exact line spans. Migration 28 `ingested_items.chunker_version`; `db::init_db` (+ backup import) re-chunks outdated items from stored text. Config probe: answer@1 0/10 → 10/10 | Standard bench unchanged (markdown-only corpora). R36.18b narrowed to byte offsets + backfill |
| R45.6 | Wizard: mode first + embeddings card (built-in first, Ollama optional, keyword-only allowed); local → RAM-sized answer-model picker (`machine.rs`, `/system/onboarding.machine`); agent → MCP or REST skill (neither preselected); source; ask/search; optional agent step. Readiness now needs an exact size tag | **No browser click-through** → R45.19. `SkillChannel` duplicates System `AgentSkillsCard` → R45.20 |
| R30.4 | One labeled `Stop <kind> #<id>` per active job (Sources row, System "Jobs", header badge); embed calls race the cancel; pending cancels instantly; 409 on repeat; `finish_job` never clobbers a cancel. Measured Stop→cancelled 24–98 ms | One big extraction has no checkpoint → R30.11. Not clicked in a browser → R45.19 |
| R30.5 | Empty-text items hash-skip; per-document tag-vector cache. Tags phase on a big re-sync 12.5 s → 0.24 s | — |
| R30.6 | `llm_chat.rs`: local Ollama answers via native `/api/chat` with `num_ctx` 8192 (`ANCHOR_OLLAMA_NUM_CTX`), 404/405 falls back to OpenAI-compatible; cloud unchanged | Streaming still a follow-up |
| R43.15 | `/backup/import` body limit 2 GiB (streams to disk) | — |
| R20.10 | `scripts/ci_mcp_search_smoke.sh` + CI step; a wrong tool name fails it | — |
| R43.31 | `frontend/src/test/setup.ts` points `localStorage`/`sessionStorage` at jsdom's (Node ≥ 25 global shadowed it). Green on Node 20/22/26 | — |
| R30.9 | `tags::threshold(conn)` reads the pass's own DB | — |
| R30.10 | Single-word tag names get their own tag unless the exact name exists (every past single-word "reuse" was a hash collision). Flat corpus 768 (saturated) → 17,377 tags | Old colliding links stay until re-processed → R30.12 |
| R35.2 | Manual PII marks gated like `sensitive` toward untrusted cloud on ask, search, `/qa/public`/`public_only`, `/decisions`, `/actions`, `/media`, sync embed, backfill, backup re-embed (`common::cloud_gate_reason`). `refusal: "gated_provider"` when nothing survives | Already-sent vectors can't be recalled (skill says mark before connecting a cloud embedder). Mock-cloud embed tests don't work on Windows (CI runs the suite on Linux only) |
| R36.15 | `GET /items/:id?lines=a-b`, `GET /nodes/:id?include=text`, 400s on malformed `path_prefix`, `read_gate.rs` (`public_only` → 404; windows/node text gated → 403). MCP `list_items`, `get_item`, `get_outline` (9 → 12 tools); skill + `docs/mcp.md` updated | **Whole-file `GET /items/:id` is still ungated** → R36.19. macOS/Windows release smoke only checks `search` |

## Waiting on the owner

1. **Sensitive content — decided 2026-10-27, not implemented (R35.4 → R35.5 → R35.6).** Owner: bundle PII into
   one **Sensitive** label (the source-form choice — the only marking UI that exists; the `pii` label and per-document
   PII toggle were removed from the UI on 2026-10-13), **gated by default on every surface, opt in to avoid** via
   per-destination Settings switches (cloud embedder / cloud answer provider / AI agents). R36.19 is closed into
   R35.5; R35.3 into R35.4. Decision entry at the end of `rust/docs/decisions.md`. Still open before R35.4 starts:
   **drop the per-document mark (default) or keep it as a per-document Sensitive mark.** Note: R35.2 (shipped in this
   batch) enforces that per-document mark — R35.4 either removes that branch or renames it. The read surfaces that
   are ungated today: `GET /items/:id` (whole file), `/sections/:id/chunks`, `/tags/:id/chunks`, `/entities/:id`
   (`window_text`), `/entities/:id/context`.
2. **Copy that still says "PII":** the R45.6 wizard hint (`AnswerModelStep.tsx`) and `AGENTS.md` (owner-decision PII
   line, label-gate line) describe the pre-bundle state — fixed by R35.4, leave until then.
3. **R30.10 — retire `tag_reuse_threshold`?** Ingest proposes single words only, so the setting now affects
   multi-word names only. Kept for now.
4. **R45.6 — answer-model RAM tiers** (`machine.rs`): llama3.2:1b any, llama3.2:3b ≥ 8 GB, qwen2.5:7b ≥ 16 GB,
   qwen2.5:14b ≥ 32 GB, qwen2.5:32b ≥ 64 GB; unknown RAM → llama3.2:3b. Also: neither agent channel preselected;
   wizard installs only the `search` skill.
5. **R36.2 choices:** boot-time synchronous re-chunk (not next sync); only depth-1/root lists count as main content;
   identifying values > 32 chars (UUIDs) stay out of the breadcrumb.

## Before tagging 1.6.0

- **R45.19 browser acceptance** on a clean `--data-dir`: local mode → RAM-sized model → sample → cited answer; agent
  mode → MCP and → skill; Ollama down → built-in embedder; two live jobs → two labeled Stops with the kept-count line;
  light + dark.
- Decide whether 1.6.0 waits for R35.4–R35.6 (item 1). No endpoint is less gated than in 1.5.0, but R36.15's new
  MCP `get_item` makes the ungated whole-file read one tool call away for agents (before, only raw REST reached it) —
  weigh that, or hold `get_item`'s whole-file form until R35.5. Either way the release notes should not advertise per-document PII enforcement (R35.2) since it is going away or
  being renamed.
- Release notes: call out migration 28 + the boot-time config re-chunk, the 12 MCP tools, PII-mark enforcement, and
  the stricter answer-model readiness (exact size tags).

## Next, by value (after the above)

R43.9 (slim scan), R43.14 (Drive OAuth refresh), R36.5, R36.8, R36.18b (byte offsets only), then the S3 group
(R43.18 / R43.19 / R43.23) that gates any networked deploy. Cheap: R45.20, R30.12, R30.11, R41.1.

## Process notes

- Parallel agents in worktrees worked well with file-disjoint rows; every merge conflict was in appended files
  (`rust/docs/decisions.md`, `AGENTS.md`, `BACKLOG.md` rows, `lib.rs` module lines) — keep both sides.
  `decisions.md` says "newest first" but recent entries are appended at the end; agents follow the tail.
- Merge onto a scratch `integrate/*` branch, run the full gates there, then fast-forward `main`.
- `gh run list --commit` needs the full SHA; use `--branch main` + run id when watching CI.
