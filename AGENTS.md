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
- Release checklist: `docs/releasing.md` (bump `rust/Cargo.toml` → `scripts/sync_version.py` → notes → tag `vX.Y.Z` → push; CI builds the three zips + `AnchorCore-macos.dmg`; the DMG is packed and checked by `scripts/make_macos_dmg.sh` + `scripts/verify_macos_dmg.sh`, shared with `build.sh`).
- macOS notarization (R41.4) — every macOS build that leaves this machine is notarized:
  - Tags: `release.yml` signs with the Developer ID (hardened runtime + `--timestamp`), notarizes + staples the app (then zips it — the update card's zip carries the stapled app) and the DMG via `scripts/notarize_macos.sh`, then `verify_macos_dmg.sh` with `ANCHOR_REQUIRE_NOTARIZED=1` (Developer ID, runtime flag, stapled tickets, `spctl` accepts). Secrets: `MACOS_CERT_P12_BASE64`, `MACOS_CERT_PASSWORD`, `MACOS_NOTARY_KEY_P8_BASE64`, `MACOS_NOTARY_KEY_ID`, `MACOS_NOTARY_ISSUER_ID`.
  - Without the cert secret the job falls back to ad-hoc with a `::warning::` — on a release that is a failed release: fix the secrets, `gh run rerun`, never ship it. After every tag confirm the macOS job printed `Gatekeeper accepts DMG + app`, or on the downloaded asset `spctl -a -t open --context context:primary-signature -vv AnchorCore-macos.dmg` → `source=Notarized Developer ID`.
  - Local builds for anyone else: `NOTARY_PROFILE=anchorcore-notary ./build.sh` (owner's keychain profile + Developer ID identity); plain `./build.sh` is ad-hoc, for this machine only. Notarizing needs network; it takes minutes.
  - A rejection prints Apple's log. Anything new in the bundle (a second executable, a dylib, a framework) must be signed inside-out with the same flags, and code that JITs or `dlopen`s unsigned libraries needs an entitlements decision — the app has none today. Rotating the cert or API key = re-set the secrets (`docs/releasing.md`). The website links the DMG for macOS; the macOS zip stays for the in-app update card (`system.rs` `update_asset_name`) and the setup skill. `rust/scripts/sync_version.py` duplicates the root script; CI `version-drift` watches it.
- CI skips docs-only pushes (`ci.yml` `paths-ignore`); tags run everything. Never add markdown the build or tests read (skills, `sample*/`, eval fixtures) to that list.

## Architecture map (`rust/crates/anchorcore/src/`)

- `main.rs` wiring (binds the port before tray init); `routes.rs` routes; `common.rs` shared helpers; `fernet.rs` secrets-file crypto — the frontend fallback stays last.
- `db.rs` open/init, `PRAGMA foreign_keys=ON`, WAL; migrations in `migrations/*.sql`, registered in `db.rs`.
- HTTP areas: `sources.rs` `entities.rs` `records_api.rs` (`/decisions`, `/actions`) `jobs.rs` `projects.rs` `items.rs` `review.rs` `settings.rs` `system.rs` `events_api.rs` (errors panel: grouped `system_events`, dismiss/restore — never deletes) `sections.rs` `tags.rs` `glossary.rs` `assets_api.rs` `captions.rs`, plus `auth.rs` `security.rs` `health.rs`. Errors are `{"detail": …}`.
- Retrieval: `answer.rs` (ask/search orchestration, follow-up rewrite, generation) + `retrieval.rs` (legs, fusion, slots, gates) + `query_expand.rs` (local-mode acronym expansion) + `vocab.rs` (glossary terms) + `llm_chat.rs` (the one local-mode chat call: native `/api/chat` + pinned `num_ctx` for a loopback Ollama, OpenAI-compatible otherwise).
- Ingest: `pipeline.rs` (`ClassifyCtx` phases) · `extract.rs` (every file format) · `diagram.rs` + `diagram_formats.rs` · `pdf_images.rs` · `chunking.rs` + `markdown.rs` + `config_chunks.rs` · `classifier.rs` (bucket + window dispatch) + `records.rs` (record parser) · `idf.rs` · `hashing.rs` · `assets.rs`.
- Models: `embedder.rs` (provider switch: built-in or HTTP) + `embed_local.rs` (candle nomic v1.5) + `model_store.rs` (pinned download, f16 convert) + `embed_model_api.rs` + `embed_fp.rs` (vector families) + `ollama.rs`. There is no classifier model.
- Ops: `watcher/` · `scheduler.rs` (source polls, embedding backfill) · `jobs.rs`.
- Platform: `secrets.rs` · `backup.rs` + `cli.rs` · `license.rs` · `frontend.rs` · `tray.rs` · `mcp.rs` + `mcp_setup.rs` · `skills_setup.rs` · `fsbrowse.rs` · `machine.rs` (RAM + answer-model catalog for the wizard).
- DB work goes through `spawn_blocking` + `with_db`/`run_db` (`pipeline.rs`); never hold a `Connection` (`!Send`) across `.await`.
- Startup (R41.7, `startup.rs`): the port binds before `init_db`; until boot finishes every route but `/health` (503 `starting`/`failed`) and the SPA shell is 503. Migrations run one transaction per version (`db::MIGRATIONS` rows carry the screen wording — a new migration needs one); background services start only after a successful boot. Debug-only test hooks: `ANCHOR_TEST_SLOW_MIGRATION_MS`, `ANCHOR_TEST_FAIL_MIGRATION`, `ANCHOR_TEST_FREE_DISK_BYTES`.

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
- **Sensitive is one concept, set per source or per document (R35.4):** source label `sensitive`, or the escalate-only per-document override `ingested_items.sensitive_override` via `PATCH /items/:id` (`true` sets, `null` clears; `false` reserved). No auto-detection or caller-asserted gate. Effective Sensitive = override OR label, derived at read time (one rule: `common::effective_sensitive` / `cloud_gate_reason`; blocked entry `item_sensitive` for the override); chunks/entities resolve via their item. Legacy `pii` labels migrate to `sensitive`.
- **Image captions are people-only (R46.12):** no suggest/auto-caption path and no MCP write tool for captions.
- **Two answer modes, one setting (R45.2):** `llm_mode` = `local` (default) | `agent`. Agent mode makes zero chat calls on the answer path and returns `mode: "agent_compose"` — which never warns (MCP included). `/qa/search` never depends on the mode. No feature may be MCP-only — the REST skill must reach everything.
- **Updates are manual-first:** plain `GET /system/update` serves cache; only `?refresh=1` or the explicit startup opt-in hits the network (`docs/updates.md`).
- **Purged names stay purged** — never write them back, not even in specs. Describe purged content by count/date; grep patterns are owner-held.

## Gotchas by area

**Database**
- `PRAGMA foreign_keys=ON` must stay — source deletes cascade through it.
- Validate before opening a write tx (single writer): a failure after the write starts stalls everything behind the scheduler.
- `RENAME COLUMN` keeps the column's constraints: to make a `try_add_column` column nullable, add a new one, copy, drop. Migration sims must build legacy columns with their real `ensure_columns` definition.
- One-shot backfills go in real migration SQL, never `ensure_columns` (re-runs every boot). Extend the v10-sim migration test with each migration.
- Multi-statement writes use a manual `BEGIN IMMEDIATE`/`COMMIT` (`run_db` hands out `&Connection`; an early `?` rolls back on drop).
- No `sqlite-vec` is linked: `vec_chunks` is a plain table and every query scans. Never recreate vec0 triggers (migration 21 removed Python-era ones); linking is R43.10.
- Empty-string `app_settings` rows mean unset: `non_empty()` reads `""` as `None`, `set("")` deletes. Never reintroduce `Some("")`.

**Secrets and security**
- Never log or return raw secrets; GET responses mask them as `***set***`.
- Stored source configs carry no secrets — create/update moves them to the SecretStore. Sync-time `resolve_config` fills *absent* fields from the store (not just `***set***` placeholders, which exist only in GET responses).
- SecretStore fallback is per key (`secrets.enc.<sha256(key)>`). `***set***`/absent = leave unchanged, explicit `""` = clear.
- Label gate: `sources.label` internal|public|sensitive, plus the per-document override (effective Sensitive, R35.4). Caller rule (R35.5, one helper `read_gate::Gate`): the SPA sends `X-Anchor-Client: ui` and is never gated; every other caller needs the person's "AI agents" switch (R35.6). One rule `sharing::allowed(settings, Destination)` for three destinations — cloud embedder, cloud answer provider, AI agents — each open when local (loopback or built-in; never agents), set by environment (`ANCHOR_CLOUD_TRUST=1` all, `ANCHOR_TRUSTED_PROVIDERS` provider hosts), or switched on in Settings → Sensitive sources; provider switches store the origin and close when the host changes; agent trust is never inferred from `answer_base_url`. Switches are `app_settings` `sensitive_share_*` rows with no env fallback, outside `PUT /settings` and backups, written only by UI-header `PUT /settings/sensitive-sharing` (audited). Single-item byte reads refuse 403 `gated_provider`, lists drop gated rows (event recorded, refusal when nothing survives); ask gates generation input on the answer switch (and also whenever the response is gated — an agent's composed answer must not carry Sensitive input) but cites everything to the UI; rewrite/expand stay provider-gated. `/qa/public` and `public_only` see `public` sources minus effective-Sensitive items, whatever the caller or provider. `/decisions` + `/actions` + `/media` + read surfaces mirror the search gate — change them together; embed gates live in ingest `embed_chunks`, the re-embed job (`fetch_embed_batch`) and the backfill SQL (`backfill_select_sql`).
- LAN bind gate: `0.0.0.0` without auth is refused on bare metal (personal opt-out `ANCHOR_LAN_NO_AUTH=1`), only warned inside containers; Team refuses everywhere.
- `POST /sources` for Jira doesn't validate the instance — only a sync fails on bad credentials.

**Ingest**
- New file types go in `extract.rs` only (folder + Drive both call it); diagrams resolve by full name first (`.drawio.svg`, `.excalidraw.json`). Legacy `.doc`/`.ppt`/`.vsd` are deliberately unsupported. Folder walking honours `.gitignore` and prunes `SKIP_DIRS`.
- Unreadable files are `skipped`, never purged.
- Folder sync: mtime+size decide whether to extract, content hash whether to re-process; the watcher compares stat listings, never content. NULL fingerprints extract once, then backfill.
- Completion is two markers: `content_hash` (stored) vs `processed_hash` (fully finished).
- Chunker dispatch: code by symbols (`doc_type == "code"`), config by key (`extract::is_config_filename`), markdown via `markdown.rs` (fence-aware blocks, line ranges, breadcrumbs). Code/config keep raw text (`skips_extraction`). B12 cleaning runs only on prose (not code, config or markdown). New splitters skip empty parents (no leading `" > "` paths).
- Chunker generations (R36.2): a splitter change that alters its output bumps that family in `chunking::chunker_version`; every chunk write stamps `ingested_items.chunker_version`, and `init_db` (plus backup import) re-chunks older items from stored text (`pipeline/rechunk.rs`) — never leave two granularities in one index.
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
- Byte reads by id (`?lines=` windows, `/nodes/:id?include=text`) go through `read_gate.rs` (`public_only` → 404, Sensitive under an untrusted provider → 403 `gated_provider`); `path_prefix` input is checked by `items::validate_path_prefix` on every surface that takes it. MCP `list_items`/`get_item`/`get_outline` forward REST keys only.
- Cited entities attribute via their item (`conflicts_for_chunks`); relationship walks used as evidence skip `conflicts_with`.
- Supersession resolves both ways: `supersedes` on the newer record marks the older one `superseded`.

**Platform and tooling**
- Logging is stdout + rotating `{data_dir}/anchorcore.log` (10 MB × 3); windowed builds only have the file.
- macOS tray needs `NSApplication` first, on the main thread (`tray.rs`); with a tray the main thread runs the platform event loop and the server lives on a `server` thread (`main.rs` `run_with_tray`) — never park the main thread in async code again (the icon goes dead). Gate with `--no-tray`/`ANCHOR_TRAY=0`. Linux tray is a stub — no tray libs in Linux CI.
- Windows builds use MinGW: `winlibs` on PATH + `cargo +stable-x86_64-pc-windows-gnu --target x86_64-pc-windows-gnu --offline` from `rust/` (MSVC can't link). Fresh worktrees lack `rust/target` and `frontend/dist` — copy or rebuild.
- PowerShell: commit with `git commit -F <file>` (a `"` in `-m` breaks args); `curl.exe -o NUL`, not `-o $null`; end CI `pwsh` steps with `exit 0` and assert statuses via `-w "%{http_code}"`.
- Line endings are LF — if an edit fails to match, suspect whitespace.
- The search skill lives in `.agents/skills/ask-docs/` and installs under a person-chosen handle: copies are found by the `<!-- anchorcore-skill: search` body marker, never by folder name — keep the marker, and keep the `name:` line + `` `/ask-docs `` example as the only handle-specific text (`skills_setup::personalize_search`).
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
