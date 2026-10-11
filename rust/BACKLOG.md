# Rust Port — Backlog (B31)

> Source of truth for incremental Rust work. Open tasks live here; completed phases 0-17 are archived verbatim in [`BACKLOG-archive.md`](./BACKLOG-archive.md). See `docs/rust-port.md` + `rust/README.md`.
>
> Product-level backlog (B-items) lives in `docs/product-plan.md` — `### B48`-`### B60` sections + the **Current execution priorities** table. Open there as of 2026-10-11: B48 (macOS + Docker legs), B49, B50, B51, B53, B55, B61. Executable B48 script: `docs/b48-clean-machine.md`.

Status: `todo` | `doing` | `done` (✅). Update this file when you pick/complete a task. Branch per task: `rust/TASK_ID`.

---

## How to pick a task (agents)

1. Claim `R*.*` by setting `doing` + branch `rust/R2.1`, etc.
2. Keep diff small, contract-first (schema → router → pipeline → tests).
3. Run `cargo test` + `PYTHONPATH=backend pytest backend/tests -q --ignore=backend/tests/test_mcp.py` against Rust on `:8123`.
4. Update this file + `AGENTS.md` when you learn a gotcha.

---

## Open work

> Current focus: Phase 25 (backup/export/device migration — 3 shipped / 4 open), Phase 20 (MCP zero-setup, 2 left) and Phase 23 (agent-friendly config, 2 left) — they share the agent-connect surfaces. Phase 18 is website-only and independent. Keep parallel tasks file-disjoint (see Files column).

### Phase 20 — MCP zero-setup UX (2026-10-11) — 2 open / 8 shipped

> One executable (`anchorcore mcp`), real-path card, one-click client connect. Shipped: R20.1-R20.3, R20.8, R20.9 (see shipped log; plan: `docs/plan-mcp-zero-setup.md`). Remaining:

| ID | Title | Priority | Est | Dependencies | DoD | Files |
|---|---|---|---|---|---|---|
| R20.4 | `runtime.json` discovery — zero env vars on any port | P1 | 0.5d | — | `main.rs` writes `~/.anchorcore/runtime.json` (port, pid, version) on boot; `src/mcp.rs` falls back to it when `ANCHOR_BACKEND_URL` is unset, stale-file-safe (pid liveness or `/health` probe). DoD: `--port 8123` + no env vars → all 6 tools work; default 8000 unchanged; error text names how to start the app when neither resolves. | `rust/crates/anchorcore/src/main.rs`, `src/mcp.rs` |
| R20.10 | End-to-end MCP search smoke in CI | P2 | 0.25d | R20.5 | The assertion R20.5 deferred: create a folder source over the repo sample/ corpus through the REST API, sync it, then drive anchorcore mcp over stdio with ANCHOR_BACKEND_URL set and require a search call to return the billing material. DoD: a deliberately broken search (wrong tool name) fails CI. Practical note from the R20.5 attempts: build the REST plumbing with curl -sS -o file -w %{http_code} plus a field extractor reading that file, and validate the idiom locally (bash -n, plus a stubbed body) before pushing — python/sed/tail pipelines on the runner hid the real failure twice | `.github/workflows/ci.yml` |

### Phase 23 — Agent-friendly configuration (2026-10-11) — 2 open / 2 shipped

> Make the tool itself configurable by an agent: shipped skill (R23.1), website surface, app-served skill, and loud one-flow BYOM answers (no silent context-only fallback).

| ID | Title | Priority | Est | Dependencies | DoD | Files |
|---|---|---|---|---|---|---|
| R23.3 | App serves its own setup skill | P2 | 0.5d | R23.1 | `GET /system/skill` returns the embedded `SKILL.md` (`text/markdown`) + a System tab "Agent setup skill" copy/download button; release smoke asserts the endpoint returns the file. | `rust/crates/anchorcore/src/system.rs`, `rust/crates/anchorcore/src/main.rs`, `frontend/src/tabs/SystemTab.tsx`, `frontend/src/api.ts`, `.github/workflows/release.yml` |
| R23.4 | Answer provider: one-flow BYOM + no silent degradation | P1 | 1d | — | B58 already adds `mode`/`retrieval` + the Ask-tab banner (`answerMode.ts`). Remaining: (a) MCP `ask` output gains a plain-language `warning` when `mode != composed` (agents must not treat context-only text as an answer); (b) Settings/Onboarding "Answers" flow: local Ollama vs the user's own OpenAI-compatible endpoint, key masked, `test-connection`, then a real compose probe that must return `mode:"composed"`; (c) `docs/mcp.md` states the BYOM contract — harness agents prefer `search` + their own model; `ask` uses the server's configured model; degraded answers stay visible, never a silent fallback to a model the user did not choose. DoD: a fresh install with no answer key cannot render context-only output without the banner + MCP warning; configuring the user's own endpoint flips the probe to `composed` in one flow. Gotcha: `answer_base_url` defaults to `https://api.openai.com/v1` — no Ollama fallback (classifier/embed do fall back); a local-only setup must set it explicitly. | `rust/crates/anchorcore/src/answer.rs`, `rust/crates/anchorcore/src/mcp.rs`, `frontend/src/tabs/SettingsTab.tsx`, `frontend/src/tabs/AskTab.tsx`, `frontend/src/OnboardingWizard.tsx`, `docs/mcp.md` |

### Phase 25 — Backup / export / device migration (Phase A of `docs/plan-backup-sync.md`) — 5 shipped / 2 open

> Portable export/import so a user can move AnchorCore to a new device or restore a backup, with secrets handled explicitly (never plaintext). Plan + phased sync design: `docs/plan-backup-sync.md` (B61). Hosted zero-knowledge sync (Phases B/C) is design-only until the §8 decisions are signed off; this phase is the shippable local slice. Branch `rust/R25.X`. Shipped: R25.1 (uids), R25.6 (secrets bundle), R25.2 (export), R25.3 (import), R25.4 (CLI) — see the shipped log.

| ID | Title | Priority | Est | Dependencies | DoD | Files |
|---|---|---|---|---|---|---|
| R25.5 | UI: System tab "Backup & restore" card | P1 | 1d | R25.2 R25.3 | Export button (options + passphrase field), file-picker import → dry-run report → confirm, "needs credentials" list, folder-path remap inputs, progress for long jobs; `npm run build/lint/test` green. | `frontend/src/tabs/SystemTab.tsx` (or `BackupCard.tsx`), `api.ts`, `types.ts` |
| R25.7 | User docs + acceptance script | P1 | 0.5d | R25.4 R25.5 R25.6 | `docs/backup.md`: export/import, secrets model, device-migration walkthrough (win/mac), restore-after-loss, format_version policy; README docs link; plan §10 A1–A3 pass on Windows + macOS. | `docs/backup.md`, `README.md` |

> Order: R25.1 + R25.6 (parallel) → R25.2 → R25.3 → R25.4 + R25.5 → R25.7 (~5d). Shipped so far: R25.1, R25.6, R25.2, R25.3, R25.4. Phases B/C (encrypted folder packs → zero-knowledge hosted/Team relay) stay in `docs/plan-backup-sync.md` §9 until Phase A ships and decisions §8 are signed off.

### Phase 18 — 18 shipped / 1 partial (deferred)

> Shipped: R18.1, R18.2, R18.16-R18.19 predate the 2026-10-11 audit; R18.3-R18.15 landed with it (R18.6 and R18.14 were partial and are now complete). R18.13 is the one partial, deferred by the owner pending user feedback. Acceptance per task: `npm run build` green in `website/`, no new hex outside `:root`, deploy via `cd website && npm run build && npx wrangler deploy`; `git grep -i acme` clean.

| ID | Title | Priority | Est | Dependencies | DoD | Files |
|---|---|---|---|---|---|---|
| R18.13 | Social proof + evidence for claims | P2 | 0.5d | — | **Done 2026-10-11:** the empty "wall of love" placeholder is gone and the FAQ latency claim links `rust/scripts/bench_retrieval.py`. **Deferred by the owner 2026-10-11** — the remaining requirement is one real trust signal, and the honest numbers (17 downloads all-time, 0 stars, 0 forks) argue the other way; the owner is collecting user feedback first. Close this by adding a real quote, or by showing a download milestone once one exists. | `website/index.html` |

> Order: R18.4 → R18.3 + R18.5 (parallel) → R18.6 (partial via R18.16) → R18.7 → R18.8 + R18.9 → R18.10 + R18.11 → R18.12-R18.14 → R18.15 last.

---

## Shipped log (2026-10-10 — 2026-10-11)

> Committed and shipped as of `v1.0.19` (2026-10-11) — the working-tree era ended with `4528b99`/`b233bea`. Full evidence lives in each commit or `docs/handover-*.md`.

| Date | Task | Shipped |
|---|---|---|
| 2026-10-11 | R25.4 | CLI `anchorcore export` / `anchorcore import` (clap subcommands, global `--data-dir/--port/--host`): prefers a running app's API (`ANCHOR_BACKEND_URL`, else 127.0.0.1:<port>, else :8000 — CSRF token fetched from `/csrf`, multipart upload, streamed download), else works on the DB directly with a locked-database hint; passphrase via `--passphrase-env` (never argv), `--yes-destructive` maps to the typed confirm, `--dry-run`, `--remap uid=path`, shell summary + exit codes. Evidence: cargo test 174/174 (4 new CLI tests incl. a real dry-run through a temp data dir), live debug-binary run: export → dry-run → merge → replace-refused-without-confirm |
| 2026-10-11 | R25.3 | Import engine + `POST /backup/import` (multipart `file` + `options`): dry run (apply-in-transaction then rollback, so conflicts are exact), merge (uid upsert + timestamp LWW, conflicts reported `imported`/`kept-local`) and replace (typed `confirm`, automatic never-swept `safety-backup-*`, data wipe in the same transaction); `needs_secret`/folder-path remap from the manifest; secrets.bundle applied under the new local ids; validation = format gate, zip-slip, decompression caps + ratio bomb guard, KDF caps; re-embed as `jobs.kind='embed'` (`Pipeline::embed_missing_for_source`, ascending-id batches, gated embedder) + local tag vectors rebuilt inline. Evidence: cargo test 170/170 (6 new import tests: round trip with secrets, dry run, replace confirm+safety, LWW keeps local, format/slip/not-a-zip refusals, foreign-embedding drop + remap), `-D warnings` clean, lizard + clippy clean for backup.rs |
| 2026-10-11 | R25.2 | Export engine + `POST /backup/export`: `.anchorcore-backup` zip (manifest.json + data.jsonl + README.txt + optional secrets.bundle) streamed with `Content-Disposition` from `exports/` (swept hourly); records rendered by SQLite `json_object` and streamed straight into the deflate entry (no accumulation), one WAL read snapshot, `backup` audit event; flags `include_secrets`/`include_embeddings`/`include_ops`/`project_id`. The plan §7 grep test is `backup::tests::export_without_secrets_has_no_plaintext_markers` (planted token + phrase; checks every decompressed entry AND the raw bytes). Evidence: cargo test 164/164, `-D warnings` clean, lizard clean for backup.rs |
| 2026-10-11 | R25.6 | Secrets bundle: Argon2id (m=64 MiB, t=3, p=1) + XChaCha20-Poly1305, versioned header bound as AEAD associated data, random salt/nonce per file, KDF caps on read (hostile-archive memory-bomb guard), keys are source uids (`source:<uid>:<field>`); wrong passphrase / header tamper / tail tamper / unknown version all fail without partial data. Format documented in `docs/backup.md` |
| 2026-10-11 | R25.1 | Stable uids (UUIDv7): `migrations/14_uids.sql` adds `uid` + unique index to sources/ingested_items/entities/chunks/sections/tags/projects, `db.rs:ensure_uid_columns` mirrors it for advanced DBs, `backfill_uids` fills NULLs at boot/export, and every insert path assigns a uid at creation. API output unchanged (uid internal until Phase B). Also fixed a pre-existing Windows-only folder-connector test failure (external_id separators). Evidence: cargo test 164/164 incl. the new backfill/stability test and the v10 live-DB migration repro |
| 2026-10-11 | R18.3 | Nav IA truth: every nav label now matches its section heading verbatim ("Roadmap" -> "Get started", section h2 renamed, #roadmap kept as the legacy anchor; "Demo" -> "60-second demo"; "Privacy" -> "Privacy notice"); demo moved to nav slot 2; burger breakpoint 760px -> 1080px (the longer labels need ~1000px; a headless render showed only ~10px slack at 900px before); README + index.html sync comments updated; verified by script + render |
| 2026-10-11 | R26.2 | --stone text contrast fixed in .matrix .n (3.31 -> 5.98:1 on the white card; text moves to --dim, --stone keeps its 9 stroke/fill uses). Survived R18.4 (scoped to --dim) and R18.8 (Lighthouse passes short cells). Verified with a precise grep for real color:var(--stone) -> 0 hits; build + typecheck + lint green |
| 2026-10-11 | R20.5 | release.yml asserts the MCP-capable binary before every upload (linux: anchorcore in the zip; macOS: the executable app binary in the bundle; windows: anchorcore.exe in the zip) and runs an initialize + tools/list handshake on the shipped file per OS, requiring 6 tools incl. search — deleting the binary now fails before upload (macOS was a soft `if [ -f ]`). ci.yml gained the same handshake smoke against the built binary. Note: `anchorcore mcp` is an argv check, not a clap subcommand, so --help is not a capability probe. **The end-to-end search smoke moved to R20.10**: four attempts died in harness plumbing, not in the product, and main was red while iterating — the handshake is the part that must hold on every push |
| 2026-10-11 | R20.7 | stdio-vs-HTTP decided and B14.2/B14.4 re-ranked: docs/mcp.md 9 (R20 row updated; **B14.2 streamable HTTP -> Next as the Team/remote path**; B14.4 registry -> Later) and 10.1 answered (local stays stdio + runtime.json discovery because a local harness must own the stdio process; HTTP removes the path for shared instances). docs/plan-mcp-zero-setup.md status + the re-rank rationale under Decisions recorded. Also repaired 7b4825a (mcp.md duplication) |
| 2026-10-11 | R20.6 | Docs + wizard audience split: docs/mcp.md 4.1 is artifact-first (setup lives in the app: System tab to Connect, the wizard card, /system/mcp for harnesses) and the dev-checkout build moved to a new section 11, For contributors, at the end; the README 60-second start gained the System-tab step. The wizard half was already satisfied - step 4 renders the shared McpConnectCard (R20.9). DoD: git grep -n cargo docs/mcp.md README.md frontend/src returns only developer-labelled hits (README Developers/Tests, mcp.md 11) |
| 2026-10-11 | R26.1 | Type-check + lint gate for website/: tsconfig.json (strict, vite/client types so ?raw imports resolve), eslint.config.js (flat + typescript-eslint), typecheck/lint scripts, allowScripts entries for esbuild/workerd, and a CI website job (node 22) between frontend and docker-team. Found and fixed three real issues on the first run: TS2551 navigator.userAgentData, TS7006 a Formspree callback param, eslint no-empty on a deliberate catch. DoD verified: a deliberate type error fails typecheck (exit 2), build still green |
| 2026-10-11 | R18.15 | Second sample corpus: sample-ai/ (10 files — a fictional AI startup: tiered model routing, eval gates, usage-tier pricing, a postmortem, and a stale 0.94 vs corrected 0.89 metric for the dispute demo), its own guide tab + #guide-ai anchor + docs/sample-ai.md, and ?corpus=ai to swap the landing examples (payments stays the static default; the task text called it ?track=, but that is the Personal/Team/Hosted switch since R18.1, so the corpus got its own param). initGuideTabs now derives tab/panel pairs from the markup. Verified: 4 tabs, deep link opens its panel with aria-expanded, corpus swaps both ways, no heading skips across 52 headings |
| 2026-10-11 | R18.13 (partial) | Removed the empty "wall of love" placeholder plus its 4 dead CSS rules, and linked the FAQ latency claim to the 30k-chunk benchmark (rust/scripts/bench_retrieval.py). The trust-signal half stays open: 17 downloads all-time, 0 stars - a real quote is needed |
| 2026-10-11 | R18.8 | a11y pass: focus rings (2px sage, paper ring on sage fills), fake tab widgets de-ARIAfied (track toggle -> aria-pressed group; guide reader -> aria-expanded/aria-controls + labelled panels), heading levels repaired (.step/.qa h4 -> h3; rendered guide markdown demoted +2 so it nests under the panel h3), phone tap targets >= 44px, initHashNav no longer underlines "How it works" on load and gained a scroll-spy with aria-current. Verified: 49 headings with no skips, role=tab/tabpanel count 0, deep link opens its panel with aria-expanded=true, no target under 44px at 390px, Lighthouse a11y 92 -> 100 |
| 2026-10-11 | R18.10 | WebMCP bridge: kept + disclosed rather than disabled. Audited the served bytes (73 KB, packs c2pa + mcp-server-client): no external URLs, no cookies/storage, no XHR/WS/beacon/postMessage; registers scan_images_c2pa + inspect_image_c2pa + the site-as-MCP-server bridge. /privacy gained an "AI agent bridge (Cloudflare WebMCP)" section; the "no third-party content" claim was dropped from the visiting bullet, the landing summary AND the meta description; README documents it as edge-injected (not a repo asset) with the dashboard toggle to disable |
| 2026-10-11 | R18.11 | Privacy notice route: the Art. 13 notice moved to its own static page (/privacy from a second Vite entry, privacy.html), linked from the footer and the form consent line; index.html keeps a two-line summary in #privacy (nav unchanged, so labels still match headings). html_handling pinned in wrangler.jsonc so the clean URL is explicit on Workers. Impressum deferred with the blocking reason recorded (5 DDG needs a publishable postal address - comment in privacy.html + README). Verified: /privacy 200 locally, 3 links to the route |
| 2026-10-11 | R18.12 | Page density: FAQ/Privacy 2-up from 900px (1607->938px), --pad-sec clamp(56px,4.4vw,148px) with a phone floor, tighter cols/steps/seal/terminal slack, TOC figure 2-column from 460px, guide reader starts collapsed (opens on click/deep link; deep-link gate still passes). 1440px: 9472 review baseline -> 10000 after the R18.6 figure rebuild -> 8030; 390px: 16433 -> 13701. The residual 530px over the ~7500 target is R18.6 height (real type), not slack - closing it needs an owner call on cutting content |
| 2026-10-11 | R18.14 | Phone type scale: <=600px floors for --fs-h1/--fs-h2 (clamp(32px,9.6vw,52px) / clamp(22px,5.2vw,28px)). Measured in a real 390px iframe viewport (Windows Chrome enforces a ~482px min window): headline 52px/4 lines -> 37.4px/2 lines; 320px -> 32px/2 lines; 600px unchanged. Meets the desktop scale at ~542px (no jump), widescreen untouched, no new tokens |
| 2026-10-11 | R18.9 | Hero copy single-source: index.html authors the default (personal) hero sub and setTrack captures it once before the first switch instead of rewriting it - no text swap on load, and crawlers index the same sentence users read. copy.personal.sub agrees with the static text; ?track=team/hosted still swap. Verified in the browser: default + ?track=personal render the static text, ?track=team renders the team copy |
| 2026-10-11 | R18.7 | Guides + Guidebook merged into one #guides section (two adjacent sections covered the same three topics; the 4-card grid orphaned a card). Now 3 guide cards - each deep-links to its panel - plus a 2-card projects/PII row (no orphan, no copy lost); the reader keeps tabs/panels and the legacy #guide-obsidian/#guide-quickstart/#guide-rest anchors (verified: each opens its panel and selects its tab). initGuidebook -> initGuideTabs, #guidebook rule dropped, .anchor no longer takes vertical space, README/design-system lists updated |
| 2026-10-11 | R18.6 | Figure legibility: both #how figures rebuilt as real HTML/CSS (was viewBox-scaled SVG at 0.69x desktop / 0.49x phone -> 5-7px labels). Measured on the built page: every label now 11.5-14px (gate >= 11px at 1440 and 390). GraphMini keeps its render-by-id API but emits HTML; narrow columns and phones stack the chain (container queries); hero-toc.svg retired; the SVG "under §3" label collision is gone; --faint no longer used for text |
| 2026-10-11 | R18.5 | Team tier conversion path: the card plain-text action became a real primary action (mailto contact via id="cta-team") plus a deploy-docs link; the hero Team CTA mirrors the card (same href + label, single source) - verified in the post-JS DOM with ?track=team. No price added on purpose: product-plan:109 keeps numbers off the site, so the contact path carries the conversion |
| 2026-10-11 | R18.4 | AA contrast on paper surfaces: `--dim` #6b7280 -> #5C6470 (paper 4.24->5.25, paper2 3.96->4.90, white 4.83->5.98); script-verified over paper/paper2/white. Audit also found `--stone` (`.matrix .n`, 3.31 on white) and `--faint` (GraphMini caption, 2.64) still below AA -> R18.6 + R18.8 |
| 2026-10-11 | R24.1 | Folder picker: `GET /system/fs` (`fsbrowse.rs`) + `FolderPicker` in the Sources form and wizard step 2 |
| 2026-10-11 | R24.2 | Business formats: one shared extractor (`extract.rs`) for folder + Drive (B9) - Word/PowerPoint/Excel/ODF/RTF/EML, encodings; Drive exports native Google types + extracts uploads; purge-safe `skipped`; 14 scratch tests; deps `zip`/`quick-xml`/`calamine`/`mailparse` |
| 2026-10-11 | R24.3 | Code + repo support: popular language extensions + Dockerfile/Makefile/.gitignore as text; folder walking honors .gitignore and skips vendor/build dirs (`ignore` crate); 12 extractor tests + repo-walk test |
| 2026-10-11 | R23.1 | `anchorcore-setup` skill (`.agents` / `.claude` / `agent` mirrors) — configure the whole tool via an agent |
| 2026-10-11 | R23.2 | Website "works with your agents" surface: dedicated Agent-skill section (setup + how-it-works terminal), canonical Connect-your-agent path → `docs/mcp.md`, skill copy/download raw link |
| 2026-10-11 | R22.2 | PDF ingestion (`pdf-extract`) + purge keeps unreadable-but-present files |
| 2026-10-11 | R22.1 | LLM classify/distill wired into the pipeline (B26 regression) + `documentation` type + note-only suppression; 138 tests |
| 2026-10-11 | R21.1 | Empty `app_settings` values read as unset — fixes flaky keyword-only/hybrid on first run (committed `333a621`) |
| 2026-10-11 | R20.9 | Wizard "Connect an agent" + Settings card: `/system/mcp/clients` + connect/disconnect for 5 clients |
| 2026-10-11 | R20.8 | One executable, two modes: `anchorcore mcp`; release zips ship one file per OS |
| 2026-10-11 | R20.3 | One-click Connect/Disconnect + Repair (shipped with R20.9) |
| 2026-10-11 | R20.2 | `GET /system/mcp` (command/args + audit counters) |
| 2026-10-11 | R20.1 | MCP card: real path + per-client command, no `cargo run` lie |
| 2026-10-10 | R19.1 | Team image: Rust single container + offline license gate |
| 2026-10-11 | R18.19 | Hero agent-skill entry point next to Download: copy-paste `npx skills add Atsirkunov/anchorcore --skill anchorcore-setup`, main CTA row + card, `#agent-skill` keeps the manual curl fallback; copy button with select-text fallback (verified live `6b5c2324`) |
| 2026-10-10 | R18.1 / R18.2 / R18.16-R18.18 | Mobile nav, sample-corpus examples, widescreen canvas, GraphMini fix, source-as-evidence |

---

## Archive — Phases 0-17 (completed)

Full tables + verification evidence: [`BACKLOG-archive.md`](./BACKLOG-archive.md).

- 0-5: port gate → skeleton → hot path → connectors → jobs → packaging (`R0.1`-`R5.3`)
- 6: cutover completeness, 121 green (`R6.1`-`R6.6`)
- 7: security hardening: origin/CSRF + route auth (`R7.1`, `R7.2`)
- 8: retrieval/RAG integrity: vectors + Unicode (`R8.1`, `R8.2`)
- 9: honest conformance + CI gate (`R9.1`, `R9.2`)
- 10: feature completeness + polish (`R10.1`-`R10.9`)
- 11: v1.0.10 review findings (`R11.1`-`R11.10`)
- 12: Stone & Sage + docs + scale corpus (`R12.1`-`R12.6`)
- 13: cyclomatic-complexity reduction, CCN 15 (`R13.1`-`R13.6`)
- 14: hierarchical TOC + tags (`R14.1`-`R14.6`)
- 15: redundancy/complexity reduction (`R15.1`-`R15.6`)
- 16: website + UI visuals (`R16.1`-`R16.8`)
- 17: provider allowlist for sensitive/PII (`R17.1`, `R17.2`)

## Links

* Eval: `docs/rust-port.md`
* Product backlog (B48+ open items): `docs/product-plan.md` → `Current execution priorities`
* Plan: `docs/product-plan.md#b31-backend-port-to-rust-p3deferred`
* Architecture: `docs/architecture.md`
* Packaging: `docs/packaging.md`
* MCP zero-setup plan: `docs/plan-mcp-zero-setup.md`
* Archive: `rust/BACKLOG-archive.md`


### Phase 26 — Website tooling follow-ups (from the Phase 18 audit, 2026-10-11) — 2 shipped

| ID | Title | Priority | Est | Dependencies | DoD | Files |
|---|---|---|---|---|---|---|

