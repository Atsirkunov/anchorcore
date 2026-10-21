# Decisions — Rust Port

ADR log. Append newest first.

---

## 2026-10-21 — R36.18a: section-granular line ranges

* **Decision:** every chunk carries the line range of its own section — markdown/verbatim report real 1-based source lines, code/config sections start at their symbol line via `push_symbol(line_no)`, prose blocks are relative to their section. Prose ranges are cleaned-text-relative (cleaning shifts lines; the mapping is documented, not stored). Joined continuation lines still report the starter line.
* **Context:** file-level ranges misattributed every citation in multi-section files. Windowed prose can't do better than section-relative without a cleaned→raw map, which stays out as over-engineering.

---

## 2026-10-21 — R43.5: Jira ADF flattened by shape, not by node list

* **Decision:** `adf_text`/`adf_node_text` walk ADF generically — block containers join children with newlines, paragraph/heading children join directly; only leaf payloads are named (`text`, `mention`/`emoji`/`media`/`status` attrs, `inlineCard` URL, `hardBreak`). Link URLs and `date` nodes are dropped (text kept). Plain-string descriptions pass through untouched; description + comments were already in `fetch`'s fields, so no new API call.
* **Context:** enumerating every ADF node type would rot with each Atlassian addition. The alternative (requesting `expand=renderedFields` HTML) trades one parser for a worse one.

---

## 2026-10-21 — R43.4: verbatim splits + para-edge trim

* **Decision:** oversized verbatim/list content splits by line (`split_verbatim_lines`, over-long lines via `split_long_line`), reusing the existing fence-close detector rather than a second one; `trim_para_edges` strips verse numbers and orphan boundary fragments from paragraph edges on the prose path. Code/config text stays byte-identical.
* **Context:** splitting keeps long code blocks and lists citable per-line instead of one giant chunk; trimming only para edges (never mid-text) avoids eating intentional content.

---

## 2026-10-21 — R43.1: scope enforced at three levels

* **Decision:** project/public scope binds the filename boost (`filename_boost_filtered`), the SQL name boundary (`named_item_ids(scope)`), and a final `scope_hit_ok` assert inside `orchestrate_fusion` — a boosted out-of-scope hit cannot survive even if an earlier leg leaks it.
* **Context:** the boost ran post-cut and pulled below-cut chunks unconditionally, so a filename match in an excluded project displaced in-scope evidence. Belt-and-braces (scoped legs + final assert) was chosen over assert-only so the legs themselves stop fetching cross-scope rows.

---

## 2026-10-21 — R43.8: default-deny auth (revises the R13 accepted table)

* **Decision:** `is_public_path` is an explicit allowlist (OPTIONS, `/health`, `/csrf`, `/auth/status|signup|login`, `/`, `/index.html`, `/assets/*`, `/favicon.svg`) — everything else requires auth when enabled. The rewrite flattened the table, so the R13 accepted-CCN-22 note is retired (now CCN 13). New rule: any new top-level static file needs an allowlist line, and `route_table_default_deny` must gain the route. Non-loopback bind without auth refuses to boot (`ANCHOR_LAN_NO_AUTH=1` opts out for personal; Team keeps its stricter no-opt-out gate).
* **Context:** the old deny-list (9 API prefixes) silently treated every unlisted GET path — `/items`, `/sections`, `/nodes`, `/tags` — as a public SPA route. The SPA is state-routed (no path routes), so `/` + assets suffice for the shell.
* **Alternatives considered:** (A) extend the prefix list — rejected, deny-listing repeats the bug on the next route. (B) serve the shell for unknown paths with auth on — rejected, reopens the hole; typed deep links 401 instead (no legit flow affected).

---

## 2026-10-21 — R43.6: nomic instruction prefixes + count check

* **Decision:** `nomic-embed-text` inputs are prefixed (`search_query:` for queries, `search_document:` for documents/backfill) when the model name contains `nomic`; other models are untouched. Embedding calls now error when the returned vector count mismatches the input count (the old `.zip` silently truncated). Stored vectors stay as-is per the R40.5 rule (note, not migration): old rows lack prefixes until force re-sync; release notes say so.
* **Context:** unprefixed nomic input is a known recall loss; a middle-dropped vector silently misaligned every later chunk. The golden eval uses hash embeddings, so it cannot see prefix effects — no silent behavior dependency there.

---

## 2026-10-21 — R43.7: answer timeout honors the existing setting

* **Decision:** generation and follow-up rewrite use the existing `answer_timeout` setting (default 120 s, was already in SETTING_KEYS/frontend but never read) instead of a new key; generation context is capped at a 6000-token estimate with byte-identical default assembly. The MCP hop gets its own budget (`ANCHOR_MCP_ASK_TIMEOUT`, default 180 s) so slow answers don't die at the proxy. num_ctx + native `/api/chat` stay with R30.6; streaming is a follow-up.

---

## 2026-10-21 — R43.3: prose-only merge + set-off CAPS rule

* **Decision:** the 150-char minimum-chunk merge applies on the prose path only (`tables` flag) — code symbols and config keys stay citable units at any size. ALL-CAPS lines section only when followed by a blank/short line, and nest (instead of popping to level 1) inside numbered sections unless preceded by a blank line. Numbered openers reading as data (dates, IPv4, bare numbers, lowercase-led remainders) never section.
* **Context:** alternatives were merging everywhere (corrupts symbol/key citations) and requiring numbered-sequence membership (drops lone legitimate headings); the blank-line escape hatch keeps set-off part headers (`APPENDIX`) top-level.

---

## 2026-10-21 — R43.2: page-boundary cleaning (conformance parity checked)

* **Decision:** repeated-line/footer stripping applies only at page boundaries (`[page N]` markers); mid-page repeats are table content. Checked against the archived suite first: `test_repeated_lines_detected_and_stripped` exercises the Python functions in-process and never touches the server, so it pins nothing about Rust behavior — no archived-mirror change (the backend is frozen anyway).

---

## 2026-10-21 — S1: no new golden-eval pairs (unit pins instead)

* **Decision:** S1 ships no new eval pairs. The harness inserts file bytes directly (no cleaning, no real splitters) and asserts presence-only recall over one scope-less source — so it cannot pin R43.1 (needs scope + a negative assertion), R43.2/R43.3/R43.4 (need cleaning/chunking), or R36.18a (ranges don't rank). Those pin at unit level (incl. a file-shaped cleaning fixture at `tests/eval_data/corpus/support/`). Bisected 2026-10-21: either candidate doc alone displaces `incident-02-midbody` from fused — that pair has zero fused margin, which blocks retrievability pins until fusion-side margin work lands (R43.27). The existing set holds 1.00 (no-delta).

---

## 2026-10-14 — R38: Markdown chunking (owner proposal)

* **Decision:** Markdown (`.md`/`.markdown`/`.mdx`) chunks by blocks, not by the prose `#`-marker path: a hand-rolled CommonMark-subset pass (`markdown.rs`, no new dependency) → heading section tree → whole-block packing with per-kind oversize rules. Markdown skips B12 cleaning only (extraction/distill still run). Embed input is breadcrumb + link-stripped text on all three embed paths; the IDF gate stays on raw stored text. Line ranges persist on sections + chunks (migration 19) and surface additively on citations/search hits/MCP/nodes; `/sections` + outline gain `depth`.
* **Context:** R36.7's `marker_heading` is fence-blind, so `#` inside fenced code blocks opened bogus sections, with no front-matter handling and no line ranges anywhere. `crates.io` is unreachable from the build box and the tree builds `--offline`, so `pulldown-cmark`/`comrak` were off the table — the subset parser covers fences, front matter, ATX/Setext, tables, lists, quotes, HTML and breaks with documented simplifications (indented code = paragraph, loose lists split at blanks, no lazy quote continuations). Partly reopens Phase 36 decision (b) the way R36.18 planned: line spans are stored (not derivable from ordering) while `prev_id`/`next_id` stay derived.
* **Alternatives considered:** (A) full CommonMark crate — rejected, can't fetch offline. (B) gate IDF on the prefixed text too — rejected, a repeated breadcrumb dilutes mean-IDF and would newly gate short-but-meaningful chunks ("Approved" under an ADR heading). (C) mass NULL + re-embed on upgrade — rejected, surprise cloud re-embed cost; old vectors stay valid and refresh on re-chunk, markdown sources need one reclassify to rebuild sections. (D) per-chunk block-kind column — rejected, packing mixes blocks so "kind" would be first-block-wins noise; section `kind` (`section`/`front_matter`) + line ranges carry the provenance instead. (E) reshape `/qa/search` into section envelopes — rejected as breaking; line ranges are additive fields. (F) wait for R36.4/R36.18 — rejected, the owner proposal explicitly asks for breadcrumb embed input + line ranges now; R38 lands the markdown-first halves (no input versioning, no byte offsets, no backfill, no bench delta — the bench needs a live server) and R36.4/R36.18 stay open narrowed (see BACKLOG).
* **Consequences:** `chunk_for_doc_type` dispatches on extension ahead of doc_type; `embed_gated` takes separate raw/embed texts; backup round-trips the new columns (old archives default `0/0`, `section`, `{}`); FTS still indexes raw content incl. link URLs (embed-input-only stripping in v1); ADR number/status/date queryable via `/sections` `meta` + the indexed front-matter chunk (no dedicated retrieval over `meta` yet).

---

## 2026-10-13 — PDF split-word catcher (adaptive extraction, not replacement)

* **Decision:** PDF extraction stays standard-first (`pdf-extract`, byte-identical for clean files) with a detect-and-repair path for the kerned/tracked PDFs that shred mid-word ("Cancellati ons"): a split-word detector scores the standard text (unknown 2–4-char fragments + rejoinable pairs against a vendored top-30k wordfreq set, `pdf_words.txt`, Apache-2.0 — see `pdf_words.NOTICE`; trip ≥6 suspects and ≥6‰, calibrated 0/832 on clean corpora); on trip a flow-layout repair pass (`PdfFlowOutput`: 0.2em word gap, explicit space breaks, `Tc`-aware advance, paragraph breaks, dehyphenation) runs and wins only on strict improvement — ties keep standard. Improved repairs warn to the log and record one `system_events` row (`pdf split-word repair`); ties stay debug-level (foreign text/typos trip the detector without being extractable damage). Same session: B12 cleaning ported prose-only (`clean_doc_text` in `store_doc_chunks`, skipped for code/config whose splitters read indentation) and `chunk_text` snaps ends to sentence ends else word breaks (bounded by overlap, tail always covered) with word-start advances clamped to the previous end — no gaps, no mid-word cuts.
* **Context:** Owner comparison found AnchorCore's PDF text shredded ("Cancellati ons", "Correcti ons", "noti fi cati ons") where a raw agent's extractor was clean — and that only *some* PDFs are affected, so blind replacement was rejected in favor of a control/catcher. First root cause: `pdf-extract`'s plain sink spaces at any glyph gap over 0.1em and ignores `Tc`/`Tw` when predicting the next glyph, so TJ kerning offsets and tracking split words; it also only checks geometry at string starts. Second root cause (found on the owner's real file, a Word-exported PDF): subsetted fonts report **width 0** for positioned glyphs (`i`, `p`, …), so every gap after one measures the full advance (0.24–0.61em) — no threshold can split it, and the 0.2em repair tied until blind gaps (<0.05em widths) were made to join unless past 1.0em. Related finds: the Rust port had dropped B12 cleaning entirely (`let cleaned = full_text.clone()`), and `chunk_text` sliced blindly mid-word/mid-sentence.
* **Alternatives considered:** (A) replace the extractor outright / fork `pdf-extract` — rejected, regression risk on good PDFs with no per-file safety signal; the adaptive design is identical-by-construction where no ambiguous gaps exist. (B) dictionary-free heuristics (short-token density) — rejected, 433/832 false trips on clean corpora (ordinary words like `walk`/`adds`/`age`); even web2 fails both ways (uninflected yet containing the true fragments `fi`/`ons`). (C) warn/event on every trip — rejected, ~1% false trips would desensitize; the improvement gate makes alerts near-zero-FP (merging real words can't reduce suspects, since correct pairs score 0). (D) clean code/config too — rejected, whitespace collapsing destroys YAML nesting and Python indentation (pinned by the config breadcrumb test).
* **Consequences:** Already-indexed bad PDFs need a force re-sync (re-extracts everything); Drive repairs are log-only (no DB handle at extraction), folder repairs also emit `system_events`. Vendored list 29015 words (~234 KB) lives in `src/` via `include_str!`. Lizard note: two behavior-identical tokenizer workarounds landed alongside (`'…'` → `'\u{2026}'`, `matches!` char alternatives → `str::contains`) — 1.24 mis-scopes both shapes; the `text_decodes_*` 592-NLOC span is a proven-cumulative artifact (CCN 1 without the PDF tests), not real complexity. Evidence: cargo test 304/307 (3 pre-existing sandbox `TcpListener::bind` EACCES in untouched answer/scheduler tests), all new fns CCN ≤ 8, synthetic kerned/tracked/hyphenated/multi-page PDF fixtures green, plus sink-level zero-width tests. Real-file verification (owner-provided Word-exported PDF, since-deleted temp probe): 2792→165 suspects, decision `repaired`, 44,570 spurious spaces removed with identical non-whitespace content chars (74401 = 74401) — residuals are domain abbrevs (SRD/IBAN-style codes) and German bits, not splits.

---

## 2026-10-13 — R36.3: group-by-parent return ("match small, return big")

* **Decision:** Ranking still matches small (leaf chunks), but the returned unit is the smallest ancestor section ≤ `ANCHOR_EXPAND_MAX_CHARS` (8 KB default) containing the match — full section bytes, every matched chunk wrapped in `[match N]` / `[/match N]` lines naming the chunk id, breadcrumb attached. Hits sharing one returned ancestor merge into a single hit (primary = first-ranked member, group sits at its rank; scores untouched). Fallback when no ancestor fits or the match is section-less: matched chunk + neighbors via the unchanged `expand_by_item` (keeps its `[continued]` form). Every search hit and citation carries `item_id`/`section_id`/`chunk_id`/`path`/`breadcrumb`/`env`/`content_truncated` (`section_id` = returned scope, `breadcrumb` = its `sections.path`, citations gain `chunk_id`); previews and ask context center on the primary mark when present (`content_window`), else keep `match_snippet`/head byte-identically. Stored `prev_id`/`next_id`/`span` links are REJECTED — siblings derive from (`section_id`, `id`) ordering (chunk ids are document order), via a recursive subtree CTE at read time.
* **Context:** Phase 36 chunking proposal (owner sign-off 2026-10-13): whole-section context beats ±2 windows for agents composing answers, and merging same-section matches removes the byte repetition the R32.9 claimed set patched over. The "smallest ancestor" rule settles the 8 KB question: tightest fitting scope wins (the direct section is eligible — excluding it would discard fitting scopes for a vacuous walk-up, since subtree sizes grow upward). The R36.6 search filters landed first and read matched-chunk identity from the DB, so grouping composes with them (post-rank AND cuts unchanged, 5x over-fetch kept).
* **Alternatives considered:** (A) largest-fitting ancestor (grow while ≤ cap) — rejected, the DoD says smallest; tightest scope keeps precision. (B) strict-parent-only (direct section ineligible) — rejected, perverse fallback when only the direct section fits. (C) backfill merged slots with next-ranked chunks — rejected for now (fewer, bigger units is the point; merging never collapses distinct files, so coverage is unaffected) — revisit if agents report thin pages. (D) stored sibling links — rejected, ordering derives them; one less thing to keep consistent on re-chunk.
* **Consequences:** `expand_by_section` ±2 + page-level `[continued]` expansion are retired (subsumed by merge); the claimed set survives for the fallback path only. Nested scopes (B's ancestor strictly contains A's) assemble fully on both sides — pinned by test, overlap accepted over holes. No schema changes. Evidence: cargo test 268/268 (15 new test fns, 3 retired), `-D warnings` clean, clippy/lizard identical to base, benchmark before/after recorded in the R36.3 report.

---

## 2026-10-12 — R34.1: System tray via tray-icon, NSApplication first on macOS

* **Decision:** Minimal tray (Open + Quit, stay-in-tray) with Tauri's `tray-icon` (+ `muda` menu), target-gated to Windows/macOS with a no-op stub on Linux so CI needs no system tray libraries. Quit drives the same graceful shutdown as Ctrl+C/SIGTERM (new `shutdown_signal`, all three select-raced). On macOS the app initializes `NSApplication` before building the status item — without it macOS 15 aborts in `CGSConnectionByID` (uncatchable assertion; tray-icon never inits it). Icon is the favicon rasterized in code (no asset). Opt-outs: `--no-tray`, `ANCHOR_TRAY=0`.
* **Context:** Owner request — windowed release builds have no console, so stopping the app meant Task Manager/Activity Monitor/`kill`. Scope locked by Q&A: minimal menu, Win+macOS, stay-in-tray (no single-instance).
* **Alternatives considered:** (A) bare `statusItemWithLength` with no NSApp (tray-icon default) — aborts on macOS 15.5, proven by a 20-line probe (exit 134) while the Swift `NSApplication.shared` + status-item probe on the same machine succeeded. (B) helper process for the tray (survive the abort) — rejected, IPC complexity for an exotic case; failures log and fall back to tray-less instead. (C) SSH-sniffing to auto-disable headless — rejected as magic; explicit flags only (true-headless macOS is untested).
* **Consequences:** `main` binds via `std::net::TcpListener` (no await) so tray creation stays on the main thread and a second instance dies at bind before planting a zombie icon; `maybe_open_browser` shares `tray::open_url`. Menu clicks can't be automated here — the seams are unit-tested (gating, icon, Notify→shutdown) and creation/serve/shutdown are live-smoked; physical clicks are hand-verified on release builds. Docs: `docs/tray.md`.

---

## 2026-10-12 — R28: Agent-native read path + ops fixes (owner bug report)

* **Decision:** Fix the owner-reported defects at the root: (1) `/qa/search` fills `item_title` and `path` (section breadcrumb, else the item's relative path via `external_id`) and accepts `include_content: true` as the alias for `fields: content` — content is served verbatim on opt-in, the summary preview grew 200→500 chars; (2) the Python embedding backfill (`scheduler.py:_backfill_loop` + startup one-shot) is ported as `scheduler.rs` `spawn_backfill`/`backfill_once` (startup + every 15 min), deliberately **without** the IDF/PII ingest gates — its job is to drain `pending_embeddings` (IDF-skipped low-signal chunks like `"The"` would otherwise stay NULL forever) and to log + record a `system_events` warning when the embedder is down; (3) tracing writes a rotating `{data_dir}/anchorcore.log` (10 MB × 3, Python parity) in addition to stdout; (4) `finish_job` no longer overwrites `total`/`processed` with the changed-items count — `total` stays the docs scanned, the result JSON's `items` stays the changed/new subset.
* **Context:** Owner session 2026-10-12: `/qa/search` hits were unusable for agents (`content`/`path`/`item_title` empty, `include_content` silently ignored — serde drops unknown fields), `pending_embeddings` sat stuck with no job or log, `anchorcore.log` held only the startup line, and sync job counts read low. Root causes: `item_title: String::new()` left in the search mapper, the Rust port dropped the backfill loop, tracing was stdout-only (invisible in windowed release builds), and the final job update wrote `total = processed = items`.
* **Alternatives considered:** (A) flip `/qa/search` to include content by default — rejected, `docs/gated-access-spec.md` A3 defines compact-by-default for context budgets; the alias makes the opt-in discoverable instead. (B) run the backfill through the ingest gates — rejected, gating is exactly what leaves the chunks stuck; `Pipeline::embed_missing_for_source` (backup import) stays gated. (C) add `tracing-appender` — rejected in favor of a small in-tree rotating writer (no new dependency, same policy as Python's handler).
* **Consequences:** Search-hit provenance is part of the contract (root `AGENTS.md` bullets); the backfill is a first-class scheduler task — do not gate or remove it; MCP `search` caps chunk bytes at 4000 chars and flags `content_truncated`, so "verbatim" is never silently truncated; user/agent surfaces no longer leak backlog ids (Projects heading, App/Ask tooltips, MCP `public_only` description). Evidence: cargo test 203/203 (5 new + the existing search compact/full test extended), `-D warnings` check clean, lizard no new warnings, `npm run build` green; live smoke on the release binary: seeded DB `pending 1→0` + `embedding backfill: embedded 1 chunks`, `include_content:true` → `item_title=m1-sprint12-retro.md`, `path=meetings\m1-sprint12-retro.md`, 3-file sync `3/3` and resync `3/3` with `items: 0`, log 1.2 KB.

---

## 2026-10-12 — R29.1: Update check is manual-first — no background pings

* **Decision:** The app never contacts the network for updates on its own: `GET /system/update` serves a cache, and performs a GitHub `releases/latest` fetch only with `?refresh=1` (60s floor). A startup check exists solely as an explicit opt-in (`update_check_enabled` / `ANCHOR_UPDATE_CHECK_ENABLED`, default off; explicit `0` beats env). No self-replace — the UI links the platform zip, and the `anchorcore-setup` skill performs the swap (Windows file-locks the running `.exe`; macOS expects a signed bundle).
* **Context:** Owner request (the setup skill must check the installer on the PC is newest) plus the trust position: an unasked-for ping to github.com would undercut "local-first". The Rust test `plain_update_get_never_checks_network` pins the contract, so merely opening the app can never phone home.
* **Alternatives considered:** (A) auto-check at boot without consent — rejected as a trust smell; (B) manual-only forever — rejected as too little notification; (C) full self-update — rejected for file-lock/signing/risk reasons (revisit if notarization ever lands).
* **Consequences:** `update_check_enabled` is a normal `app_settings` key (DB wins over env, like every other setting); the banner is per-version dismissible; `anchorcore --version` (clap `command(version)`) lets the setup skill check an on-disk binary without booting it; docs live in `docs/updates.md`.

---

## 2026-08-29 — Phase 13: Cyclomatic complexity ≤ 15 CCN as a standing design rule

* **Decision:** Every function in `rust/crates/anchorcore/src` must stay ≤ 15 CCN (lizard, `-C 15`); `is_public_path` (`auth.rs:216`, CCN 22) is the single accepted decision table and is explicitly do-not-touch. This is now a standing design rule, not a one-off cleanup — see `rust/AGENTS.md` "Cyclomatic complexity" + root `AGENTS.md` workflow step 0.
* **Context:** A 2026-08-28 lizard sweep found 24/329 functions warned, worst `classify_and_store` CCN 73. Phase 13 (`rust/BACKLOG-archive.md`) reduced all of them in 6 file-disjoint tasks: `pipeline.rs` `ClassifyCtx` phases + `with_db`/`run_db` boilerplate helper; `system.rs` `error_row` + dynamic `params_from_iter` (4 identical match arms → 1); connector parse helpers (`parse_issue`, `parse_file`, `graphql_error`); handler helper extraction across `entities.rs`/`pii.rs`/`review.rs`; `main.rs` `build_router`/`build_state` phases + watcher `fetch_and_compare`; `answer.rs` `llm_rewrite`/`heuristic_rewrite` + retrieval row converters (`hit_from_row10`, `fallback_hit_from_row8`).
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

## 2026-10-12 — R35.1: people-driven PII — manual doc mark, auto-detect deleted

* **Decision:** PII is marked by people at whole-document level (`ingested_items.is_pii`, `PATCH /items/:id`, Entities toggle, `item_is_pii` on entity JSON, backup round-trip). The auto-detect stack is deleted, not deprecated-in-place: `pii.rs` incl. `scan_text` + the regex KB, `/pii/*` routes, ingest `flag_pii` phase, `embed_gated` chunk flags, the `?gate=provider` honor system, and the chunk-`is_pii` half of the answer/search gate. Source labels + `public_only` + cloud-trust stay and are now the only enforcement.
* **Context:** Owner decision after the direct-API bypass report: entity/item reads gated only on self-asserted `?gate=provider`, and ask/search gated against the server's provider instead of the byte consumer. Building a real multi-principal trust model for a single-user local app is overengineering; the honor system gave false confidence. The auto machinery also had measured costs (`flag_pii_for_item`: 28 s on one PDF, 100–250 ms per small file — R30.5) and regex detection leaks by nature.
* **Alternatives considered:**
  - *A: `scan_text` survives as a suggest-helper* — rejected: no suggest UI exists and none is planned; dead code with a KB to maintain. Git history preserves it if a suggest feature is ever specified.
  - *B: Keep chunk flags, gate on them* — rejected: same leak-by-nature problem; doc-level people marks are coarser but truthful.
  - *C: Fix `?gate` into real enforcement* — rejected: the bypass report's option set; overengineering at this stage (see Context).
* **Consequences:**
  - `chunks.is_pii`/`pii_categories` columns stop being written but stay until a follow-up drop migration (R35.3); backup keeps round-tripping them meanwhile.
  - How manual marks gate cloud classify/embed/answer is a follow-up ticket (R35.2), not bundled — until then marks are advisory + visible.
  - R30.5's "PII scan bounded" clause is resolved by deletion; its tag/empty-text clauses stand.

---

## 2026-10-19 — R40.5: no silent mass re-embed for pre-1.0.28 bare vectors

* **Decision:** Vectors embedded before R38.2 (1.0.28) keep their bare content-only bytes until a force re-sync re-chunks the item (which NULLs embeddings; the re-embed then uses heading-prefixed input). The scheduler backfill fills only NULL embeddings, likewise prefixed. No version-gated mass re-embed.
* **Context:** Since R38.2 every embed path embeds `embed_input(chunks.path, content)` uniformly — the R40.5 filing premise ("PDFs embed bare") holds only for empty-path chunks (undetected structure → fallback windows) and for vectors stored before 1.0.28. A mass re-embed would re-bill/slow every upgrade for a gain most corpora get incrementally; force re-sync is the explicit refresh. Golden pair `rfp-escrow-01` pins the prefix gain on the vector leg.
* **Consequences:** Recently synced sectioned docs carry prefixed vectors; older ones converge on next force re-sync. Any future embed-input format change reuses this rule (note, not migration) unless recall measurably requires otherwise.

---

## 2026-10-19 — R40.7: quarantined corpus vocabulary

* **Decision:** Corpus jargon extracts at ingest into `vocabulary` (term, expansion, item, source, origin, status) and expands FTS terms at query time. Tier-1 rules (parenthetical acronyms anywhere, `CODE - x` / `Term: def` under glossary headings) go active immediately; Tier-2 LLM pairs (`defined_terms` piggyback on classify) stay proposed until corroborated by a second independent item (same-item re-store never self-corroborates), confirmed by any rule row, or approved in R40.8 review. Activation is sticky; `rejected` rows survive re-sync and suppress the pair globally. Rejected rows witness nothing.
* **Context:** RFP-style docs define codes the query never spells (`Long Form (CODE)`); curated synonyms (R40.6) cannot cover per-corpus jargon. LLM pairs are quarantined because a single model guess is the least trustworthy signal in the pipeline — the golden pair `dunning-01` proves the expansion path while unit tests pin the quarantine. No-model installs get Tier 1 only (pure rules, no classify dependency). Structured files (code/config) extract nothing, consistent with the extraction skip.
* **Alternatives considered:**
  - *A: LLM pairs active immediately* — rejected: single-witness model output expanding every matching query is how "CA" learns to mean something wrong corpus-wide.
  - *B: Per-item rejection scope* — rejected: a rejected pair is wrong, period; global suppression is simpler and matches "never re-proposed".
* **Consequences:**
  - Backup archives do not carry `vocabulary` yet (format change out of R40.7 scope) — restore + force re-sync rebuilds it; file the backup-table follow-up if restores go routine.
  - Query-time lookup is project-scoped but label-blind (terms, not content); chunk label/status gates apply downstream as before.

---

## 2026-10-21 — R44.19 / R44.20: no website analytics yet; drop the WebMCP bridge

* **Decision:** (R44.19) No analytics, no download-redirect Worker for now — deferred until there is traffic worth measuring. Per-OS download totals come from GitHub's per-release-asset counts; waitlist/quote intent comes from the Formspree `interest` field. (R44.20) Turn the Cloudflare-injected WebMCP bridge (`/.webmcp/bridge.js`) off; agent discoverability is served by `website/public/llms.txt`.
* **Context:** The site's strongest trust claim is "no cookies, no analytics, no third-party requests, only our own scripts". Web Analytics would add a beacon and cannot see downloads (they leave for GitHub); a counting Worker adds a failure point in the download path. The bridge is ~73 KB of edge-injected code we do not control (audited once, 2026-10-11), exposes generic image-provenance tools, and is the one script on the site that is not ours.
* **Alternatives considered:**
  - *A: Cloudflare Web Analytics* — rejected for now: breaks the no-analytics wording for visit/referrer data that is not decision-relevant at current traffic; revisit if visit→download conversion becomes the question.
  - *B: `/download/{os}` Worker + counter* — rejected for now: per-day OS trends are not worth a new moving part in the download path; GitHub asset counts already give per-OS totals.
  - *Keep the bridge* — rejected: unaudited updates, no AnchorCore-specific value, contradicts the only-our-scripts claim.
* **Consequences:**
  - Owner action: Cloudflare dashboard → Agent Readiness → WebMCP → off. Until it is flipped, the privacy notice's bridge disclosure stays; after, remove the disclosure from `website/privacy.html` and `website/README.md` in the same change as confirming the served HTML has no bridge script.
  - If analytics returns, update the privacy notice + README "no analytics" statement in the same change (R44.19 acceptance).

---

## 2026-10-21 — R44.9 / R44.16: positioning + Team pricing posture

* **Decision:** The public voice is "private AI search" with the hero "Ask your files. Get the answer and the exact source." ("memory", "second brain", "knowledge base" retired from the site). Team is quote-only: `/team` says "Request a quote" and no price, range, license model or support-fee structure appears on the site, the root README or `hosting/README.md`. The fee structure is undecided (owner exploring a fixed license plus an annual support fee); nothing is published until it is chosen.
* **Context:** The previous docs promised a "one-time, perpetual license incl. 1 year of support/updates". The owner has not landed on that; publishing it would pre-commit pricing. The license *mechanism* (offline-verified key, maintenance expiry warns and runs) is unchanged in code and still documented factually in `hosting/README.md`.
* **Alternatives considered:**
  - *Publish a price anchor/range on `/team`* — rejected: no number decided.
  - *Keep the one-time-license wording* — rejected: contradicts the undecided structure.
* **Consequences:**
  - `docs/product-plan.md` and `docs/v2v3-scope.md` keep their original pricing rows as history, annotated as superseded.
  - `LICENSE.md` is legal text and is unchanged ("separate paid license"); revisit with counsel once pricing is chosen.
  - The in-app Ask tab heading still says "Ask your memory" — separate follow-up.

---
