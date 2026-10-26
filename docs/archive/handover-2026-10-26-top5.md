# Handover 2026-10-26 — top-5 backlog, what's left

Read root `AGENTS.md` first (gates, gotchas, owner decisions). This page holds only the state of the five rows picked
as highest-value on 2026-10-26 and what remains of them. Row text in `rust/BACKLOG.md` stays the source of truth;
this page is context.

## Outcome (same day, shipped in 1.5.0)

All of the top five are done: R45.3/R45.4 (built-in embedder; owner chose
"`auto` prefers a serving Ollama" after measuring ~5× faster bulk indexing),
R30.2 + R43.12 (concurrency kept for remote APIs; vector reuse is the real
re-sync win), R41.2 (owner's clean-Mac acceptance passed). The cheap bundle
shipped too (R45.14, R45.10, R30.3, R43.17, R20.11, R41.6), plus R43.20 and
R47.0/R47.1 from a parallel session. Evidence and numbers are in the
backlog rows and `rust/docs/decisions.md`. Next by measurement: the tags
phase on re-sync (R30.5).

## State of `main` at handover

- Pushed to `origin/main`, **not tagged** (still 1.4.1): `db19078` Cargo.lock 1.4.1 sync · `f9259ba` merge R43.11 ·
  `41194e7` merge R20.4 · `5445923` merge R41.2 · `925cf5f` R41.5 · this handover. The next release tag ships all of
  it (bump per `docs/releasing.md`).
- Gates on the merged tree: `ANCHOR_SECRETS_NO_KEYRING=1 cargo test -p anchorcore` 667 passed / 0 failed,
  `RUSTFLAGS="-D warnings" cargo check --tests` clean, clippy clean, lizard = the 7-warning baseline.
- Agent worktrees and the merged `rust/R43.11`, `rust/R20.4`, `rust/R41.2` branches were removed (local only, never
  pushed). The `.muse/worktrees/*` and `r40-work` worktrees predate this session — not touched.

## Shipped today (don't redo)

| Row | What landed | Known gaps (filed or noted) |
|---|---|---|
| R43.11 | Extractor `catch_unwind` → file `skipped`; `finish_guarded` runs every job body under a panic guard so `finish_job` always runs; `promote_lock()` recovers a poisoned `PROMOTE_LOCK` | Panics inside `wait_for_slot` / `finish_job` themselves are outside the guard (DB-open failures only) |
| R20.4 | `runtime.rs`: app writes `{data_dir}/runtime.json` after bind, removes its own on graceful shutdown; `anchorcore mcp` resolves env → health-checked file → `:8000`, re-resolves on connect failure | Connect writes only `["mcp"]` → filed **R20.11** (pass non-default `--data-dir`). Windows unverified |
| R41.2 | `scripts/make_macos_dmg.sh` + `verify_macos_dmg.sh` (CI + `build.sh`), `AnchorCore-macos.dmg` in `release.yml` + `SHA256SUMS`, site Mac button → DMG; zip still ships (update card + setup skill use it); `build.sh` heredoc bug fixed | **Clean-Mac acceptance still open** (see below). Website must not be deployed by hand before a tag that carries the DMG — `releases/latest/download/AnchorCore-macos.dmg` 404s until then |
| R41.5 (unplanned, owner bug) | Tray was dead on macOS since R34.1: the main thread sat in `axum::serve`, AppKit never ran. `main` is now sync; with a tray the server runs on a `server` thread and the main thread pumps `NSApplication.run` / a Win32 `GetMessageW` loop. Owner confirmed icon + Open + Quit | 10-min Activity Monitor "Responding" check not run; **Windows tray runtime unverified** (first Windows CI/MinGW build — check it); icon hides under long menus on notched Macs → filed **R41.6** (template icon) |

## Remaining from the top 5, in order

### 1. R45.3 → R45.4 — built-in embedder (biggest adoption win: no Ollama prerequisite)

- Owner decisions are made (2026-10-21, `docs/archive/handover-2026-10-21-llm-modes.md` §7.2): **nomic-embed-text v1.5,
  f16 on disk, built-in first**. Don't re-ask.
- Evidence so far: `docs/probes/2026-10-21-embedding-engine.md` — candle 0.11 `models::nomic_bert`, parity ≥ 0.99995 vs
  Ollama on every backend incl. f16 and Linux aarch64, Metal f16 fastest, pure-CPU f16 ≈ Accelerate f32 (so Accelerate
  can likely be dropped), Matryoshka 512-d lossless on the small eval. §7 lists the implementation implications (f16
  storage, U8 mask, F32 pooling, length-sorted batches).
- **The probe source was never committed** (it lived in a scratchpad). Rebuild it from §1 + §6 of the probe doc if you
  need the CI legs; better, fold the CI legs into R45.4's own CI job instead of resurrecting the probe.
- R45.3's open legs are CI-only: Windows MinGW build (`x86_64-pc-windows-gnu`, `--offline` — every new crate must be in
  `Cargo.lock`), x86 Linux (with/without `mkl`), and an Ollama CPU-only throughput baseline to compare against
  (~3 chunks/s built-in on Linux CPU vs ~4.5 min/10k chunks Ollama-on-Metal). Record results in the probe doc + a
  `rust/docs/decisions.md` entry, then close R45.3.
- R45.4 DoD (row): `embed_provider` setting, model download manager with checksum + resume and an offline drop path,
  per-OS cargo features (Metal on macOS, pure CPU elsewhere), Team image prefetch in `hosting/Dockerfile`, Ollama
  provider still works. A no-Ollama machine must get hybrid retrieval after the first-run download.
- Must-keep contracts: vectors compare only within a family — stamp `chunks.embed_fp` via `Embedder::sync_vec` and add
  the built-in family to `SHARED_FAMILIES` only if parity really makes them one space (`embed_fp.rs`); nomic
  `search_query:` / `search_document:` prefixes (R43.6); `embed_input(path, content)` on all three embed paths; the
  15-min backfill stays ungated; sensitive/pii label gate only matters for cloud providers (built-in is local).
- Unblocks: R45.6 (wizard RAM-sized picker), R43.20 (close it if the Team image embeds built-in).
- Files: `embedder.rs`, new `embed_local.rs`, `Cargo.toml` (target-gated features), `hosting/Dockerfile`, CI.
  **Conflicts with item 2** (`embedder.rs`) — run them in sequence or with one agent.

### 2. R30.2 + R43.12 — ingest speed (one agent, same `pipeline.rs`)

- Measured bottleneck (R30.7, 2026-10-23): 82 s of a 91 s file was 263 back-to-back embed batches. Today
  `Embedder::embed` loops `texts.chunks(BATCH_SIZE=32)` sequentially and builds a new `reqwest::Client` per call;
  `embed_chunks` in `pipeline.rs` embeds one document's pending chunks at a time.
- R30.2: fill batches across chunks/documents and/or keep a bounded number of batches in flight (env knob, default ≥ 2,
  capped); per-document phase order kept; DB writes stay in `run_db`. R43.12: clean + chunk **before** `BEGIN IMMEDIATE`
  (a large-PDF sync must not starve writers past `busy_timeout`), and reuse embeddings on an exact `(path, content)`
  match with no migration — a one-char edit elsewhere in the doc must not re-embed unchanged chunks.
- Record before/after on the same 8401-chunk file and the 600-file corpus in the rows (the R30.7 numbers are the
  baseline). If built-in lands first, measure with both providers.
- R43.11 just wrapped job bodies in `finish_guarded` — new job kinds or restructured bodies must still go through it.

### 3. R41.2 — clean-Mac acceptance (owner, manual)

1. Tag a release; confirm the Release has `AnchorCore-macos.dmg`, the three zips and `SHA256SUMS`; the CI
   `build-rust-macos` log shows `ok (macos dmg): …` (actionlint was never run locally on the workflow).
2. On a clean Mac or fresh user profile: site button → DMG → drag to Applications → eject → launch → Open Anyway
   (not notarized, R41.4) → `curl -s http://127.0.0.1:8000/health` ok → `ps` path is `/Applications/AnchorCore.app/…`
   with no `AppTranslocation`; tray icon visible (look left of existing icons) with Open/Quit working.
3. Same run: B48 §3 (`docs/b48-clean-machine.md`) + backup A1–A3 (`docs/backup.md`); record results there and in the
   R41.2 row, then mark it done. The owner's own Mac already passed steps 2's install/launch/tray legs (not clean).

## Cheap bundle to run alongside (file-disjoint from items 1–2 unless noted)

- **R45.14** (`classifier.rs`, 0.25d): `RECORD_PATTERNS` has no action-items/checkbox marker — such docs bucket
  `reference` and yield zero records. Keep the parser strict.
- **R45.10** (migration 27, 0.5d): demote model-era entities to `unverified`, delete rule rows from non-record items;
  back up first, record counts; extend the v10-sim migration test.
- **R30.3** (`watcher/service.rs`, 0.5d): fallback loop still ticks every 1 s and calls `init_db` per folder source.
- **R43.17** (`retrieval.rs` + `main.rs`, 0.25d): `sort_hits` uses `partial_cmp().unwrap()` (NaN panic);
  `maybe_open_browser` still runs before the bind — note `main.rs` now has `run_with_tray`/`serve` (R41.5).
- **R20.11** (`mcp_setup.rs`/`system.rs`, 0.25d) and **R41.6** (`tray.rs`, 0.25d) — today's follow-ups.

## Housekeeping noticed, not done

- Owner's machine has an old `~/Applications/AnchorCore.app` next to the new `/Applications` one; MCP client configs
  still launch the old path until Connect is clicked again (owner action, not code).
