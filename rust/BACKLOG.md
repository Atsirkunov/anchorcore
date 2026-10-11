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

> Current focus: Phase 20 (MCP zero-setup, 4 left) and Phase 23 (agent-friendly config, 3 left) — they share the agent-connect surfaces. Phase 18 is website-only and independent. Keep parallel tasks file-disjoint (see Files column).

### Phase 20 — MCP zero-setup UX (2026-10-11) — 4 open / 5 shipped

> One executable (`anchorcore mcp`), real-path card, one-click client connect. Shipped: R20.1-R20.3, R20.8, R20.9 (see shipped log; plan: `docs/plan-mcp-zero-setup.md`). Remaining:

| ID | Title | Priority | Est | Dependencies | DoD | Files |
|---|---|---|---|---|---|---|
| R20.4 | `runtime.json` discovery — zero env vars on any port | P1 | 0.5d | — | `main.rs` writes `~/.anchorcore/runtime.json` (port, pid, version) on boot; `src/mcp.rs` falls back to it when `ANCHOR_BACKEND_URL` is unset, stale-file-safe (pid liveness or `/health` probe). DoD: `--port 8123` + no env vars → all 6 tools work; default 8000 unchanged; error text names how to start the app when neither resolves. | `rust/crates/anchorcore/src/main.rs`, `src/mcp.rs` |
| R20.5 | CI: assert the MCP-capable binary in zips + per-OS Inspector smoke | P1 | 1d | R20.8 | `release.yml` asserts the MCP-capable binary exists on every OS (macOS `if [ -f ]` → hard fail) and a per-OS smoke launches the app + `npx -y @modelcontextprotocol/inspector --cli ./anchorcore mcp --method tools/list` expecting 6 tools + one `search` on `sample/`. DoD: deleting the binary from a test build fails CI before upload. | `.github/workflows/release.yml`, `ci.yml` |
| R20.6 | Docs + wizard audience split | P1 | 1d | R20.3 R20.9 | `docs/mcp.md` setup artifact-first (dev `cargo` box at the bottom for contributors); `OnboardingWizard` final step reuses the R20.3 card; `README.md` quick-start points at the System tab. DoD: `git grep -n cargo docs/mcp.md README.md frontend/src` shows no user-facing build step. | `docs/mcp.md`, `frontend/src/OnboardingWizard.tsx`, `README.md` |
| R20.7 | stdio vs HTTP decision + B14.2/B14.4 re-rank | P2 | 0.25d | R20.6 | Record the local default (stdio + R20.4 discovery) vs B14.2 streamable HTTP in `docs/mcp.md` §9/§10 and re-rank B14.2/B14.4 (HTTP removes the path entirely for Team; registry = one-click for Claude users). DoD: milestones table updated, open question §10.1 answered. | `docs/mcp.md`, `docs/plan-mcp-zero-setup.md` |

### Phase 23 — Agent-friendly configuration (2026-10-11) — 2 open / 2 shipped

> Make the tool itself configurable by an agent: shipped skill (R23.1), website surface, app-served skill, and loud one-flow BYOM answers (no silent context-only fallback).

| ID | Title | Priority | Est | Dependencies | DoD | Files |
|---|---|---|---|---|---|---|
| R23.3 | App serves its own setup skill | P2 | 0.5d | R23.1 | `GET /system/skill` returns the embedded `SKILL.md` (`text/markdown`) + a System tab "Agent setup skill" copy/download button; release smoke asserts the endpoint returns the file. | `rust/crates/anchorcore/src/system.rs`, `rust/crates/anchorcore/src/main.rs`, `frontend/src/tabs/SystemTab.tsx`, `frontend/src/api.ts`, `.github/workflows/release.yml` |
| R23.4 | Answer provider: one-flow BYOM + no silent degradation | P1 | 1d | — | B58 already adds `mode`/`retrieval` + the Ask-tab banner (`answerMode.ts`). Remaining: (a) MCP `ask` output gains a plain-language `warning` when `mode != composed` (agents must not treat context-only text as an answer); (b) Settings/Onboarding "Answers" flow: local Ollama vs the user's own OpenAI-compatible endpoint, key masked, `test-connection`, then a real compose probe that must return `mode:"composed"`; (c) `docs/mcp.md` states the BYOM contract — harness agents prefer `search` + their own model; `ask` uses the server's configured model; degraded answers stay visible, never a silent fallback to a model the user did not choose. DoD: a fresh install with no answer key cannot render context-only output without the banner + MCP warning; configuring the user's own endpoint flips the probe to `composed` in one flow. Gotcha: `answer_base_url` defaults to `https://api.openai.com/v1` — no Ollama fallback (classifier/embed do fall back); a local-only setup must set it explicitly. | `rust/crates/anchorcore/src/answer.rs`, `rust/crates/anchorcore/src/mcp.rs`, `frontend/src/tabs/SettingsTab.tsx`, `frontend/src/tabs/AskTab.tsx`, `frontend/src/OnboardingWizard.tsx`, `docs/mcp.md` |

### Phase 25 — Backup / export / device migration (Phase A of `docs/plan-backup-sync.md`)

> Portable export/import so a user can move AnchorCore to a new device or restore a backup, with secrets handled explicitly (never plaintext). Plan + phased sync design: `docs/plan-backup-sync.md` (B61). Hosted zero-knowledge sync (Phases B/C) is design-only until the §8 decisions are signed off; this phase is the shippable local slice. Branch `rust/R25.X`.

| ID | Title | Priority | Est | Dependencies | DoD | Files |
|---|---|---|---|---|---|---|
| R25.1 | Stable uids (UUIDv7) on syncable tables | P0 | 0.5d | — | migration `14_uids.sql`: `uid TEXT UNIQUE` on sources, ingested_items, entities, chunks, sections, tags, projects (disputes/merge_actions keyed by uid refs; backfill for existing rows); `db.rs:migrations()` updated; API output unchanged (uid internal until Phase B); `cargo test` green. | `rust/crates/anchorcore/migrations/14_uids.sql`, `src/db.rs` |
| R25.6 | Encrypted secrets bundle format | P0 | 0.5d | — | `secrets.bundle`-style format documented (`docs/backup.md`): Argon2id (m=64MiB, t=3, p=1) + XChaCha20-Poly1305, per-file salt/nonce, versioned header; used by export and (Phase C) the account-key envelope; unit tests for wrong passphrase + tamper detection. | `src/backup.rs` (new), `docs/backup.md` |
| R25.2 | Export engine + `POST /backup/export` | P0 | 1d | R25.1 R25.6 | streams `.anchorcore-backup` (zip: manifest.json + data.jsonl + README.txt); record types per plan §3; secrets excluded by default with `needs_secret` markers; `include_secrets` + passphrase → `secrets.bundle`; `include_embeddings`/`include_ops` flags; consistent snapshot while the app runs; export audit event; streaming-safe. DoD test: planted token + marker phrase absent from all plaintext bytes. | `src/backup.rs`, `src/main.rs`, `Cargo.toml` |
| R25.3 | Import engine + `POST /backup/import` | P0 | 1d | R25.2 | `dry_run` report (counts, format gate, `needs_secret`, folder path remaps, conflicts); `merge` (uid upsert + LWW) / `replace` (typed confirm + automatic safety backup); transactional; zip-slip + decompression caps; re-embed job for imported chunks missing vectors; refuse newer `format_version`. | `src/backup.rs`, `src/main.rs`, `src/pipeline.rs` |
| R25.4 | CLI: `anchorcore export` / `anchorcore import` | P1 | 0.5d | R25.2 R25.3 | headless/Team path; talks to the running API (single DB owner) or opens the DB directly when the app is stopped (clear error when locked); `--include-secrets --passphrase-env` / `--replace --yes-destructive` / `--dry-run`; shell summary. | `src/main.rs` (clap subcommand), `src/backup.rs` |
| R25.5 | UI: System tab "Backup & restore" card | P1 | 1d | R25.2 R25.3 | Export button (options + passphrase field), file-picker import → dry-run report → confirm, "needs credentials" list, folder-path remap inputs, progress for long jobs; `npm run build/lint/test` green. | `frontend/src/tabs/SystemTab.tsx` (or `BackupCard.tsx`), `api.ts`, `types.ts` |
| R25.7 | User docs + acceptance script | P1 | 0.5d | R25.4 R25.5 R25.6 | `docs/backup.md`: export/import, secrets model, device-migration walkthrough (win/mac), restore-after-loss, format_version policy; README docs link; plan §10 A1–A3 pass on Windows + macOS. | `docs/backup.md`, `README.md` |

> Order: R25.1 + R25.6 (parallel) → R25.2 → R25.3 → R25.4 + R25.5 → R25.7 (~5d). Phases B/C (encrypted folder packs → zero-knowledge hosted/Team relay) stay in `docs/plan-backup-sync.md` §9 until Phase A ships and decisions §8 are signed off.

### Phase 18 — Landing-page audit + example content (`website/`) — 9 open / 2 partial / 7 shipped

> Shipped: R18.1, R18.2, R18.16-R18.18 (see shipped log). R18.6 and R18.14 are partial — their notes say what is left. Acceptance per task: `npm run build` green in `website/`, no new hex outside `:root`, deploy via `cd website && npm run build && npx wrangler deploy`; `git grep -i acme` clean.

| ID | Title | Priority | Est | Dependencies | DoD | Files |
|---|---|---|---|---|---|---|
| R18.5 | Team tier conversion path | P1 | 0.25d | — | The Team pricing card's only action ("Deploy via Docker — self-hosted") is plain text, no price appears anywhere on the site, and the hero's team CTA points at `hosting/README.md` while the pricing card points nowhere. DoD: Team card has a working primary action plus a price or "from €X / seat" and a contact path; hero team CTA and pricing card share one target. | `website/index.html`, `website/src/main.ts` |
| R18.6 | Figure legibility at real sizes | P1 | 0.5d | R18.2 | `hero-toc.svg` is a 720×220 diagram whose labels are 9–12px inside the viewBox, drawn into a ~498px column in `#how` `.duo` → ~6–8px effective; `GraphMini` is a fixed-height 190px 640×190 SVG that letterboxes inside its card. DoD: effective label size ≥ 11px at 1440 and at 390 (render the figures full-width, enlarge the internal type, or rebuild as HTML/CSS with real type); `npm run build` green. Graph instance rebuilt + capped in R18.17 (0.95× at 1440, 1.19× at 2560); flat `hero-toc.svg` still scales to ~0.69× at 1440 (≈1.2× at 2560 via R18.16) and both are ~0.68× on phones. | `website/public/hero-toc.svg`, `website/src/components/GraphMini.ts`, `website/src/style.css` |
| R18.7 | Merge Guides + Guidebook | P1 | 0.5d | — | Two adjacent sections cover the same three topics (Obsidian, sample company, JSON API) under two headings; the 4-card grid orphans one card on its own row and only "Guides" is in the nav. DoD: one section with one heading, each card deep-linking to its guide tab; nav updated; legacy `#guide-obsidian`/`#guide-quickstart`/`#guide-rest` anchors still work. | `website/index.html`, `website/src/main.ts` |
| R18.8 | a11y pass — focus, tabs, headings, tap targets | P1 | 0.5d | — | No `:focus`/`:focus-visible` rule exists anywhere in `style.css`; `role="tab"`/`role="tabpanel"` are used without `aria-controls`/`aria-labelledby` or arrow-key handling (track toggle + guide tabs); FAQ and Privacy skip `h2 → h4`, and guide panels render `h3` above markdown `h2`s; tap targets are 27px (track toggle) and 24px (terminal Replay) vs the 44px guidance; `initHashNav` underlines "How it works" on load (default `#how`) and never updates on scroll. DoD: visible focus ring on every interactive element; tabs either completed or de-ARIAfied to plain buttons with `aria-expanded`; heading levels monotonic; mobile targets ≥ 44px; Lighthouse a11y ≥ 95. | `website/src/style.css`, `website/index.html`, `website/src/main.ts` |
| R18.9 | Hero copy single-source | P1 | 0.25d | — | `index.html` hero sub ("Turn folders, Drive, Jira and Linear into a private AI knowledge base…") and `copy.personal.sub` in `main.ts` ("Your notes, PDFs, Jira, Linear and Drive…") already disagree, so users see a text swap after load and crawlers index the other sentence. DoD: one source for the default copy (inject at build time, or stop swapping the default track); `?track=` deep links and `localStorage["anchorcore.track"]` unchanged. | `website/index.html`, `website/src/main.ts` |
| R18.10 | Edge-injected `/.webmcp/bridge.js` — decide + disclose | P1 | 0.5d | — | Every visit loads `/.webmcp/bridge.js` (21KB — the largest asset on the page, `data-packs="c2pa,mcp-server-client"`), injected at the Cloudflare edge (WebMCP): `git grep webmcp` finds nothing in the repo and `wrangler.jsonc` has no config, while the privacy notice states "no third-party content". DoD: decision recorded (keep with disclosure, or disable); if kept, audit what it exposes and add a line to the notice; note the behaviour in `website/README.md` so it is not mistaken for a repo asset. | `website/README.md`, `website/index.html` |
| R18.11 | Privacy notice → `/privacy` + Impressum | P1 | 0.5d | — | The Art. 13 notice is an in-page section (1,194px desktop) rather than a linkable route, and `website/README.md:43` still lists Impressum as pending a publishable postal address — a gap for an EU-facing paid product. DoD: `/privacy` route (static, linked from the footer and the form consent line) with the landing page keeping a two-line summary; imprint published, or the deferral recorded with the blocking reason. | `website/index.html`, `website/src/main.ts`, `website/README.md` |
| R18.12 | Page density and length | P2 | 0.5d | R18.7 R18.11 | Desktop page is 9,472px tall: 12 sections at 104px vertical padding each, FAQ (1,393px) and Privacy (1,194px) are long card stacks, and three full guides sit in the DOM with one visible. DoD: under ~7,500px desktop with no copy lost; FAQ shortens (accordion or two columns); section padding scales down below 640px. | `website/index.html`, `website/src/style.css` |
| R18.13 | Social proof + evidence for claims | P2 | 0.5d | — | "Wall of love — opening soon" ships empty; the latest release assets show 2 macOS / 3 Windows / 0 Linux downloads; the FAQ latency claim had no supporting link (reworded 2026-10-10 to the measured 30k-chunk benchmark + projection). DoD: at least one real trust signal on the page (download count or a user quote — **not** GitHub stars while the repo sits at 0, see R18.18) and the latency claim links the benchmark (`rust/scripts/bench_retrieval.py`); empty wall-of-love placeholder removed until quotes exist. | `website/index.html`, `website/src/main.ts` |
| R18.14 | Responsive type scale | P2 | 0.25d | R18.1 | `h1` is a flat 52px (four-line headline at 390px), `h2` 28px, and `.wrap` keeps 32px gutters on a 390px viewport. DoD: `clamp()` type scale, headline ≤ 3 lines at 390px, 16–20px gutters on phones; no new tokens. Done for gutters + condensed header in R18.1 (`@media(max-width:420px)`); phone type scale still open (`h1` is still a flat 52px at 390px — the widescreen clamps deliberately keep the low end unchanged). | `website/src/style.css` |
| R18.15 | Second sample corpus (AI-startup track) | P3 | 1d | R18.2 | The payments corpus is the only concrete example set, while the page also sells to AI/engineering teams. DoD: a second bundled corpus (`sample-ai/`: model routing, evals, pricing decisions) with its own guide tab, landing examples selectable per `?track=`; the payments examples stay the default. | `sample-ai/*`, `website/src/main.ts`, `website/index.html`, `docs/` |

> Order: R18.4 → R18.3 + R18.5 (parallel) → R18.6 (partial via R18.16) → R18.7 → R18.8 + R18.9 → R18.10 + R18.11 → R18.12-R18.14 → R18.15 last.

---

## Shipped log (2026-10-10 — 2026-10-11)

> Committed and shipped as of `v1.0.17` (2026-10-11) — the working-tree era ended with `4528b99`/`b233bea`. Full evidence lives in each commit or `docs/handover-*.md`.

| Date | Task | Shipped |
|---|---|---|
| 2026-10-11 | R18.3 | Nav IA truth: every nav label now matches its section heading verbatim ("Roadmap" -> "Get started", section h2 renamed, #roadmap kept as the legacy anchor; "Demo" -> "60-second demo"; "Privacy" -> "Privacy notice"); demo moved to nav slot 2; burger breakpoint 760px -> 1080px (the longer labels need ~1000px; a headless render showed only ~10px slack at 900px before); README + index.html sync comments updated; verified by script + render |
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

