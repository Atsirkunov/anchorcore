# AnchorCore — agent guide (living file)

Read this before doing anything. It holds what a fresh session needs so it doesn't re-explore the repo: **live contracts only** — no line numbers, no phase history (that lives in `rust/BACKLOG.md` / `rust/BACKLOG-archive.md` / `rust/docs/decisions.md`), no war stories. Add a line when you learn something that would have helped at the start; delete lines that stop being true.

## Session start

- `git fetch origin`; compare `HEAD` with `origin/main` (`git log --oneline HEAD..origin/main`, `git tag --sort=-v:refname | head`). Releases and other sessions move `main` — reconcile before writing code.
- `backend/` (Python) is archived on tag `archive/python-final`; nothing uses it. All work is Rust.

## What this is

Local-first "company memory": connect sources (folder, Jira, Linear, Drive, Miro, REST) → extract and chunk with provenance → hybrid search and cited Q&A for people and agents. One Axum process serves the React SPA. Stack: **Rust (Axum) + SQLite (FTS5, embedding scan)** · React/Vite · built-in candle embedder (nomic v1.5, one explicit download), Ollama or BYO OpenAI-compatible for embeddings and local-mode answers. Ships as a single binary (`frontend/dist` embedded); `hosting/` is the one-container Team image. Version: `rust/Cargo.toml` (`workspace.package.version`). Repo: `git@github.com:Atsirkunov/anchorcore.git`. System view: `docs/architecture.md`.

## Run / test / release

- Dev: `cargo run -p anchorcore -- --port 8000 --data-dir ~/.anchorcore`.
- Gates for every change: `cargo test -p anchorcore` (unit + black-box `tests/api_*.rs` + golden eval) green, `RUSTFLAGS="-D warnings" cargo check` clean (clippy `unwrap_used` is allowed crate-wide), lizard no new warnings (baseline + rules in `rust/AGENTS.md`), and for UI changes `cd frontend && npm run build`.
- On a dev Mac run tests with `ANCHOR_SECRETS_NO_KEYRING=1` (otherwise two tests read your real keychain). The suite must pass without Ollama.
- The binary embeds `frontend/dist` at build time: after a UI change run `npm run build`, then `cargo build --release`.
- Team image: `docker compose -f hosting/docker-compose.yml up --build`; license gate `ANCHOR_EDITION=team` (`license.rs`, issue with `python scripts/make_license.py`). Compile-time includes outside `rust/` (`include_str!`/`include_dir!`, e.g. `.agents/skills/`) must also be copied in `hosting/Dockerfile` — audit with `grep -rn "include_\(str\|dir\|bytes\)!" rust/crates/anchorcore/src` before tagging.
- Release checklist: `docs/releasing.md` (bump `rust/Cargo.toml` → `scripts/sync_version.py` → notes → tag `vX.Y.Z` → push; CI builds the three zips + `AnchorCore-macos.dmg`, ad-hoc signed, not notarized; the DMG is packed and checked by `scripts/make_macos_dmg.sh` + `scripts/verify_macos_dmg.sh`, shared with `build.sh`). The website links the DMG for macOS; the macOS zip stays for the in-app update card (`system.rs` `update_asset_name`) and the setup skill. `rust/scripts/sync_version.py` duplicates the root script; CI `version-drift` watches it.
- CI skips docs-only pushes (`ci.yml` `paths-ignore`); tags run everything. Never add markdown the build or tests read (skills, `sample*/`, eval fixtures) to that list.

## Architecture map (`rust/crates/anchorcore/src/`)

- `main.rs` wiring (binds the port before tray init); `routes.rs` routes; `common.rs` shared helpers; `fernet.rs` secrets-file crypto — the frontend fallback stays last.
- `db.rs` open/init, `PRAGMA foreign_keys=ON`, WAL; migrations in `migrations/*.sql`, registered in `db.rs`.
- HTTP areas: `sources.rs` `entities.rs` `records_api.rs` (`/decisions`, `/actions`) `jobs.rs` `projects.rs` `items.rs` `review.rs` `settings.rs` `system.rs` `sections.rs` `tags.rs` `glossary.rs` `assets_api.rs` `captions.rs`, plus `auth.rs` `security.rs` `health.rs`. Errors are `{"detail": …}`.
- Retrieval: `answer.rs` (ask/search orchestration, follow-up rewrite, generation) + `retrieval.rs` (legs, fusion, slots, gates) + `query_expand.rs` (local-mode acronym expansion) + `vocab.rs` (glossary terms).
- Ingest: `pipeline.rs` (`ClassifyCtx` phases) · `extract.rs` (every file format) · `diagram.rs` + `diagram_formats.rs` · `pdf_images.rs` · `chunking.rs` + `markdown.rs` · `classifier.rs` (bucket + window dispatch) + `records.rs` (record parser) · `idf.rs` · `hashing.rs` · `assets.rs`.
- Models: `embedder.rs` (provider switch: built-in or HTTP) + `embed_local.rs` (candle nomic v1.5) + `model_store.rs` (pinned download, f16 convert) + `embed_model_api.rs` + `embed_fp.rs` (vector families) + `ollama.rs`. There is no classifier model.
- Ops: `watcher/` · `scheduler.rs` (source polls, embedding backfill) · `jobs.rs`.
- Platform: `secrets.rs` · `backup.rs` + `cli.rs` · `license.rs` · `frontend.rs` · `tray.rs` · `mcp.rs` + `mcp_setup.rs` · `skills_setup.rs` · `fsbrowse.rs` · `machine.rs` (RAM + answer-model catalog for the wizard).
- DB work goes through `spawn_blocking` + `with_db`/`run_db` (`pipeline.rs`); never hold a `Connection` (`!Send`) across `.await`.

## Retrieval (core rules)

`ask()`: scope `source_ids` → `rewrite_followup` (pronoun-only, local mode) → `orchestrate_fusion` (TOC-or-flat legs → RRF → age decay → cap/dedupe → live-first → authority tiers → cut + type slots → keyword slots → filename boost → relevance gate) → `generate_answer` (local mode) or `agent_compose` (agent mode). Knobs and the reason for each fix: `docs/guides/retrieval-tuning.md`; TOC: `docs/guides/hierarchical-toc.md`.

- RRF only: `score = Σ weight/(60+rank)`. Never fuse raw scores; demote/reorder after fusion. Hits carry leg provenance and RRF-range scores — never raw BM25 next to RRF.
- Locked-out floor hits are fixed by slot reservation, never by re-tuning weights (TOC@1.0 vs floors@0.5 is structural). ask, search and the eval share `orchestrate_fusion`.
- `status_ok` (`retrieval.rs`) is the single stale/disputed filter for every path, including `/decisions` and `/actions`. New status exclusions go there only. `ANCHOR_QA_EXCLUDE_DISPUTED` toggles disputed.
- Every retrieval fn takes `source_ids` (`None` = everything).
- Final hits are leaf chunks with match-centred previews; summary chunks are TOC internals and never surface. Sibling expansion never repeats bytes.
- Test environments: live-first ordering (`prefer_live_envs` stable-partitions before every cut). Env comes from path markers and fixture-grade file-head vocabulary only. Queries naming test files are exempt; explicit filenames beat env.
- Golden eval: `cargo test -p anchorcore --test retrieval_eval` — fused recall stays 1.00 on every pair (`tests/eval_data/README.md`). It tests leg plumbing with hash embeddings, not model quality.

## Owner decisions — never reintroduce without a new decision entry

- **Ingest is model-free (R45.18):** no chat call on the ingest path — no LLM extraction, distillation or classifier settings. `extract_entities` is `rules` (default) or `off`; `code` skips extraction. Prose understanding is the agent's job at question time.
- **Records only from structure (R45.8):** `Decision:` headings/fields, `Status:`, `Owner:`, ADR front matter, `Action items` bullets, checkboxes — never prose, table rows or fenced code. Extraction goes through `Classifier::classify` per window (routing via `classify_rules` once fabricated records). Extraction stores `unverified`; **`verified` means a person confirmed it** — no confidence auto-promotion.
- **Vocabulary is people-owned (R45.13):** origins `manual` (`/glossary`), `glossary` (a file marked `is_glossary`), `rule` (`Full Phrase (FP)` passing `is_initialism`). No LLM terms, proposal status or review queue. Agent mode skips `rule` rows.
- **PII is a manual whole-document mark (R35.1):** `ingested_items.is_pii` via `PATCH /items/:id`. No auto-detection or caller-asserted gate. Marks are advisory until R35.2.
- **Image captions are people-only (R46.12):** no suggest/auto-caption path and no MCP write tool for captions.
- **Two answer modes, one setting (R45.2):** `llm_mode` = `local` (default) | `agent`. Agent mode makes zero chat calls on the answer path and returns `mode: "agent_compose"` — which never warns (MCP included). `/qa/search` never depends on the mode. No feature may be MCP-only — the REST skill must reach everything.
- **Updates are manual-first:** plain `GET /system/update` serves cache; only `?refresh=1` or the explicit startup opt-in hits the network (`docs/updates.md`).
- **Purged names stay purged** — never write them back, not even in specs. Describe purged content by count/date; grep patterns are owner-held.

## Gotchas by area

**Database**
- `PRAGMA foreign_keys=ON` must stay — source deletes cascade through it.
- Validate before opening a write tx (single writer): a failure after the write starts stalls everything behind the scheduler.
- One-shot backfills go in real migration SQL, never `ensure_columns` (re-runs every boot). Extend the v10-sim migration test with each migration.
- Multi-statement writes use a manual `BEGIN IMMEDIATE`/`COMMIT` (`run_db` hands out `&Connection`; an early `?` rolls back on drop).
- No `sqlite-vec` is linked: `vec_chunks` is a plain table and every query scans. Never recreate vec0 triggers (migration 21 removed Python-era ones); linking is R43.10.
- Empty-string `app_settings` rows mean unset: `non_empty()` reads `""` as `None`, `set("")` deletes. Never reintroduce `Some("")`.

**Secrets and security**
- Never log or return raw secrets; GET responses mask them as `***set***`.
- Stored source configs carry no secrets — create/update moves them to the SecretStore. Sync-time `resolve_config` fills *absent* fields from the store (not just `***set***` placeholders, which exist only in GET responses).
- SecretStore fallback is per key (`secrets.enc.<sha256(key)>`). `***set***`/absent = leave unchanged, explicit `""` = clear.
- Label gate: `sources.label` internal|public|sensitive|pii. Localhost providers are trusted; cloud needs `ANCHOR_TRUSTED_PROVIDERS` or `ANCHOR_CLOUD_TRUST=1`. Sensitive/pii content is withheld from untrusted cloud embed/answers (event recorded, refusal when nothing survives) and skips follow-up rewrite. `/qa/public` and MCP `public_only` see only `public` sources. `/decisions` + `/actions` mirror the search gate — change both together.
- LAN bind gate: `0.0.0.0` without auth is refused on bare metal (personal opt-out `ANCHOR_LAN_NO_AUTH=1`), only warned inside containers; Team refuses everywhere.
- `POST /sources` for Jira doesn't validate the instance — only a sync fails on bad credentials.

**Ingest**
- New file types go in `extract.rs` only (folder + Drive both call it); diagrams resolve by full name first (`.drawio.svg`, `.excalidraw.json`). Legacy `.doc`/`.ppt`/`.vsd` are deliberately unsupported. Folder walking honours `.gitignore` and prunes `SKIP_DIRS`.
- Unreadable files are `skipped`, never purged.
- Folder sync: mtime+size decide whether to extract, content hash whether to re-process; the watcher compares stat listings, never content. NULL fingerprints extract once, then backfill.
- Completion is two markers: `content_hash` (stored) vs `processed_hash` (fully finished).
- Chunker dispatch: code by symbols (`doc_type == "code"`), config by key (`extract::is_config_filename`), markdown via `markdown.rs` (fence-aware blocks, line ranges, breadcrumbs). Code/config keep raw text (`skips_extraction`). B12 cleaning runs only on prose (not code, config or markdown). New splitters skip empty parents (no leading `" > "` paths).
- Embedding inputs are `embed_input(path, content)` on all three paths; the IDF gate reads raw content — keep them split.
- Re-chunking keeps vectors: `store_doc_chunks` copies `embedding` + `embed_fp` onto new chunks with an exact `(kind, path, content)` match (summaries via `restore_vectors`). Anything that changes what `embed_input` reads must change that key too. Clean + chunk run before `BEGIN IMMEDIATE`. `ANCHOR_EMBED_CONCURRENCY` (default 2) only helps remote providers — local Ollama is compute-bound.
- Every chunk-vector write stamps `chunks.embed_fp` (`Embedder::sync_vec` does it); vectors are compared only within a family (`SHARED_FAMILIES` is an explicit allowlist). The backfill (startup + 15 min) is deliberately ungated; backup-import re-embed stays gated.
- PDF extraction is adaptive: the repair pass runs only when the split-word detector trips and wins only on strict improvement.
- Job concurrency: `MAX_CONCURRENT=2`, excess queues as `pending`. At most one sync/reclassify per source — explicit POSTs get 409 with the live `job_id`, watcher/scheduler triggers coalesce. Job `total` is set by the run phase, not `finish_job`.
- Job bodies run through `Pipeline::finish_guarded` (`jobs::guard_job` catches panics → `failed`, slot freed); a new job kind must too. Extractor panics are caught in `extract.rs` → file skipped. Take `PROMOTE_LOCK` via `promote_lock()` (poison-recovering), never `.lock().unwrap()`.

**Images (assets)**
- Bytes live in the DB (`asset_blobs` + `assets`, migration 26) for every source kind — never store a path or fetch later. Relative image links resolve only inside the canonical folder root; no network on ingest. Asset uids are deterministic (item uid + src + ordinal) so re-sync keeps them. Over `asset_max_bytes` → metadata row with NULL bytes + one warning, never a sync failure.
- Office/ODF/PDF images are `embedded:` markers resolved from the doc package at sync (Drive: the downloaded bytes on `IngestionDoc.package`, never a second download). PDF: drawn image XObjects only, never rasterize.
- Served at `GET /media/:uid` (DB only, label-gated, CSP-sandboxed). Never mount API routes under `/assets/` — that is the public SPA bundle dir.
- Captions are derived `kind='caption'` chunks rebuilt from `assets.manual_*`; sync never overwrites manual attribution. Every leaf-kind filter lists `'document','caption'` — new ones must too.

**Backup** (`docs/backup.md`): uid-based (never local ids), one-transaction import (dry-run = apply then rollback), secrets bundle decrypted before any write, `exports/safety-backup-*` never swept, chunk→section links deferred to the end of the stream (`link_deferred_sections`). New tables/columns must round-trip.

**Agent surfaces**
- `/qa/search` hits carry provenance and opt-in verbatim content (`include_content` ≡ `fields: content`; MCP caps 4000 chars + `content_truncated`). `path` = section breadcrumb, else the item's `external_id`.
- Item discovery is `GET /items` (metadata only), drill-down `GET /items/:id`; payloads expose `content` (`text` is an alias). Entities carry `chunk_id` next to `item_id`.
- Cited entities attribute via their item (`conflicts_for_chunks`); relationship walks used as evidence skip `conflicts_with`.
- Supersession resolves both ways: `supersedes` on the newer record marks the older one `superseded`.

**Platform and tooling**
- Logging is stdout + rotating `{data_dir}/anchorcore.log` (10 MB × 3); windowed builds only have the file.
- macOS tray needs `NSApplication` first, on the main thread (`tray.rs`); with a tray the main thread runs the platform event loop and the server lives on a `server` thread (`main.rs` `run_with_tray`) — never park the main thread in async code again (the icon goes dead). Gate with `--no-tray`/`ANCHOR_TRAY=0`. Linux tray is a stub — no tray libs in Linux CI.
- Windows builds use MinGW: `winlibs` on PATH + `cargo +stable-x86_64-pc-windows-gnu --target x86_64-pc-windows-gnu --offline` from `rust/` (MSVC can't link). Fresh worktrees lack `rust/target` and `frontend/dist` — copy or rebuild.
- PowerShell: commit with `git commit -F <file>` (a `"` in `-m` breaks args); `curl.exe -o NUL`, not `-o $null`; end CI `pwsh` steps with `exit 0` and assert statuses via `-w "%{http_code}"`.
- Line endings are LF — if an edit fails to match, suspect whitespace.
- Skills: only `.agents/skills/` (our own) is tracked; vendored skills are restored from `skills-lock.json` — never `git add -A` them back. Both of our skills are embedded in the binary (`include_str!`); a test pins them byte-identical.

## Feature workflow

1. Schema: migration `N+1` in `migrations/*.sql`, registered in `db.rs`; extend the v10-sim test.
2. API: handler + route (frontend fallback last); extend `tests/api_*.rs` when shared behaviour changes.
3. Wire into `retrieval.rs` / `pipeline.rs` if it affects retrieval or ingest.
4. Frontend: `types.ts` + `api.ts` + the tab; colours only via `theme.ts` tokens (`var(--ac-*)` from `theme.css`, light + dark) — no inline hex, no left-border stripes (`docs/design-system.md`).
5. Gates (above), plus the golden eval.
6. Docs in the same commit: `rust/BACKLOG.md` row → done, the relevant `docs/` page, `rust/docs/decisions.md` when you chose between real options.
7. CCN ≤ 15 per function — split orchestrators into named phase helpers.

## UI notes

- `App.tsx` tabs + header (project picker scopes Ask); `OnboardingWizard.tsx` (steps in `wizard/`) on first launch with no sources: mode + embeddings (built-in first) → local: RAM-sized answer model (`/system/onboarding` `machine`) | agent: MCP or REST-skill channel → folder or bundled `sample/` → cited answer. `FolderPicker.tsx` browses via `GET /system/fs` — keep the typed-path input.
- Ask renders newest-first via `AnswerView.tsx` + `answerText.ts` (strips `[match N]`, safe markdown subset, `[S1]` marks). `SourcesTab` composes `tabs/sources/*`; job polling is the single `useJobsPoll` hook; projects live in `ProjectsContext`.
- `api.ts request()` throws `Error(detail)`, 30 s timeout, `RequestAbortedError` on cancel; `types.ts` mirrors backend schemas; error boundaries per tab.
- Website: CI deploys anchorcore.dev on `v*` tags when `website/` changed; manual `cd website && npm run build && npx wrangler deploy`. Landing copy uses only the `sample/` or `sample-ai/` corpora. `/.webmcp/bridge.js` is edge-injected, not a repo asset.
