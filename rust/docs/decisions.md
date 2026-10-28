# Decisions — Rust Port

ADR log. Append newest first.

---

## 2026-10-28 — R35.6: per-destination Sensitive opt-in, host-bound, UI-only

* **Decision:** one rule `sharing::allowed(settings, Destination)` for three destinations — cloud embedder, cloud answer provider, AI agents (MCP / skill). Each opens when local (loopback or the built-in embedder; never agents), set by environment (`ANCHOR_CLOUD_TRUST=1` opens all three; `ANCHOR_TRUSTED_PROVIDERS` opens matching provider hosts only), or the person's switch is on. Provider switches store the destination **origin** at opt-in and close when the configured host changes (the card reports the stale consent). Switches are `app_settings` rows `sensitive_share_*` with no env fallback, outside `PUT /settings` and backups, written only by `PUT /settings/sensitive-sharing` with the UI header; every change writes a `settings` system event (on = `warning`, off = `info`). Agent trust is no longer inferred from `answer_base_url`. In local mode, an agent caller without the agents switch gets an answer whose generation input is gated too (`ask_gates`), so a composed answer cannot carry Sensitive text to the agent.
* **Context:** R35.5 left env-only trust and inferred agent trust from the answer provider (a local Ollama answer model opened every agent read). With separate destinations, that inference stopped being true: the answer switch and the agents switch mean different things.
* **Alternatives considered:**
  - *Boolean switches not bound to a host* — rejected: pointing the embedder or answer model at a new host would silently carry the old consent ("per destination" would be per kind, not per destination).
  - *Switches in the generic `PUT /settings` / `SETTING_KEYS`* — rejected: any agent with the CSRF token could opt itself in, the change would not be audited, and `ANCHOR_<KEY>` env fallback would add a third, invisible override.
  - *Round-trip switches in backups like other settings* — rejected: consent is given on this machine, in its UI, with an audit row; an archive must not grant it silently.
  - *A migration with a dedicated table* — rejected: three keys fit `app_settings` (empty = unset).
  - *Keep agents open when the answer provider is local* — rejected by the row spec; also the agent is a separate recipient whatever model answers.
* **Consequences:** behaviour change on upgrade: agents (MCP/skill) that read Sensitive content because the answer model was a local Ollama now get `gated_provider` until the person turns on "AI agents". The UI-only write is a guardrail against well-behaved agents, not a boundary (a local process can call with the header or edit the DB), consistent with R35.5's scope.

---

## 2026-10-26 — R45.6: wizard picks the answer model by RAM; explicit size tags must match exactly

* **Decision:** `GET /system/onboarding` gains `machine` = `{total_memory_bytes, recommended_answer_model, answer_models[]}` (`machine.rs`: `sysctlbyname("hw.memsize")` on macOS, `/proc/meminfo` capped by cgroup v2 `memory.max` on Linux, `GlobalMemoryStatusEx` on Windows; `null` elsewhere). The catalog lives server-side so the setup skill can read the same recommendation: `llama3.2:1b` (any RAM), `llama3.2:3b` (8 GB), `qwen2.5:7b` (16 GB), `qwen2.5:14b` (32 GB), `qwen2.5:32b` (64 GB); the recommendation is the largest whose floor the machine meets with 0.5 GiB firmware slack, and `llama3.2:3b` (the old default) when RAM is unknown. Model readiness (`ollama.rs`) now requires an explicit non-`latest` tag to be installed exactly; untagged/`:latest` names still match any tag of the family. The wizard is mode-first: step 1 sets the mode and search embeddings (built-in model first, Ollama `nomic-embed-text` offered as the faster alternative when Ollama runs); local mode then picks the answer model, agent mode picks a channel (MCP or the REST skill, neither preselected).
* **Context:** B54's family-only match made a configured `qwen2.5:14b` read "ready" when only `qwen2.5:7b` was installed, so a size picker would have reported success and every answer would 404. The R45.17 note that the wizard's missing-models line wasn't mode-aware is resolved by splitting it: embeddings and answer models are checked in their own steps.
* **Alternatives considered:**
  - *Catalog in the frontend, RAM only from the server* — rejected: the REST/skill setup path would have no recommendation (no feature UI-only).
  - *Add the `sysinfo` crate* — rejected: a new dependency tree for one number; the three platform calls need no new crate (`libc` was already in the lock, `windows-sys` gains one feature).
  - *Exact matching for every name* — rejected: `nomic-embed-text` configured vs `nomic-embed-text:v1.5` installed must keep working.
* **Consequences:** model choices (qwen2.5 for 16 GB+) are a product call to revisit as Ollama's catalog moves; only `machine.rs` changes. Windows RAM detection compiles only in CI (MinGW).

---

## 2026-10-21 — R45.2: two answer modes (`llm_mode` agent/local), `agent_compose`

* **Decision:** one setting, `llm_mode` = `agent` | `local` (default `local`, so upgrades keep today's behavior). In `agent` mode `ask` / `POST /qa` / `POST /qa/public` keep scope + `public_only` + the label gate + fusion but skip `rewrite_followup` and `generate_answer` — zero chat calls — and return `mode: "agent_compose"`: a one-line compose instruction as `answer`, citations always with content, and no warning (nothing degraded). `answer_ready` is true in agent mode (`answer::answer_ready`); `/system/status` reports `llm_mode`. `/qa/search` is mode-independent. The agent has two equal channels — MCP and the REST `anchorcore-search` skill — and no feature may be MCP-only. A BYO cloud answer endpoint is an advanced variant of `local` on the same code path, not a third mode.
* **Context:** agents already compose from `search`; routing their `ask` through the server's model warned "not composed" when no model existed, which read as broken. Alternatives were three modes (agent/local/cloud — the cloud path is code-identical to local) and inferring the mode from "no answer model configured" (silently flips on a misconfiguration). The agent-mode gate reuses the existing provider-trust verdict so `ask` and `search` can never disagree; whether the agent itself counts as an untrusted provider is owner decision 7.2.5 (handover 2026-10-21), and would change both together. The UI Ask tab renders agent mode as an evidence view with a neutral hint — the proposed default for open decision 7.2.4.

---

## 2026-10-21 — R43.29: the LAN gate warns inside containers (revises R43.8)

* **Decision:** `lan_bind_gate` takes `in_container` (`/.dockerenv` or `/run/.containerenv`): personal-edition containers boot with a loud warning naming the host-side `-p` control instead of refusing. Bare-metal personal still refuses; Team still refuses everywhere with no opt-out. The probe itself is deliberately unit-untested (asserting it would fail for anyone running `cargo test` in a container); the pure matrix pins the logic.
* **Context:** 1.0.33 refused every personal container boot (CI `ac-smoke` curl exit 7) because the image must bind 0.0.0.0 to reach the host mapping. Alternatives were a CI-only env var (leaves every personal-docker user bricked) and downgrading the gate to warn-everywhere (surrenders the bare-metal protection); the carve-out keeps both.

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
* **Consequences:** Already-indexed bad PDFs need a force re-sync (re-extracts everything); Drive repairs are log-only (no DB handle at extraction), folder repairs also emit `system_events`. Vendored list 29015 words (~234 KB) lives in `src/` via `include_str!`. Lizard note: two behavior-identical tokenizer workarounds landed alongside (`'…'` → `'\u{2026}'`, `matches!` char alternatives → `str::contains`) — 1.24 mis-scopes both shapes; the `text_decodes_*` 592-NLOC span is a proven-cumulative artifact (CCN 1 without the PDF tests), not real complexity. Evidence: cargo test 304/307 (3 pre-existing sandbox `TcpListener::bind` EACCES in untouched answer/scheduler tests), all new fns CCN ≤ 8, synthetic kerned/tracked/hyphenated/multi-page PDF fixtures green, plus sink-level zero-width tests. Real-file verification (owner-provided Word-exported PDF, since-deleted temp probe): 2792→165 suspects, decision `repaired`, 44,570 spurious spaces removed with identical non-whitespace content chars (74401 = 74401) — residuals are domain abbrevs (product codes) and German bits, not splits.

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
  - *A: LLM pairs active immediately* — rejected: single-witness model output expanding every matching query is how "DD" learns to mean something wrong corpus-wide.
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

---

## 2026-10-21 — R43.28: a lone ordinary word in the file head must not classify a doc as test

* **Decision:** `env_for_item` head markers are two tiers. Strong (`testdata`, `mock`, `mocks`, `fixture`, `fixtures`): one occurrence in the first 1 KB still flags the doc. Weak (`sandbox`, `staging`): need two distinct weak words. Path-segment markers (`sandbox/`, `test/`, ...) are unchanged, as are the query-names-test-file exemption and `ANCHOR_TEST_ENV_DEMOTE=1.0`.
* **Context:** On the shipped `sample/` corpus the billing decision ("a proper sandbox") and runbook ("Provision sandbox") classified `env: test`; live-first ordering then put them behind every live doc (rank 6-7 of 7, m2-q3-planning absent at k=8). "Sandbox" is normal vocabulary in payments docs, so real users hit this.
* **Alternatives considered:**
  - *Require >=2 distinct markers for all head words* — rejected: breaks the pinned R32.8 behavior that a single `mock`/`fixture` head flags seed data, which is the case head detection exists for.
  - *Drop `sandbox`/`staging` from the head list entirely* — simpler, but loses `# sandbox + staging tenant seed` style headers; the two-word rule keeps them.
  - *Stored per-item env / per-node env (R36.8)* — out of scope, larger.
* **Consequences:**
  - Env is computed at query time from the item path and `substr(text,1,1024)` (`chunk_item_env`), nothing is persisted, so no migration and no re-sync: existing indexes are fixed on upgrade.
  - Residual: a live doc whose head says `mock` or `fixture` once still reads as test; accepted (strong tier), revisit only on a real report.
  - Regression pin: `adr-012-billing` in the golden eval carries a lone "sandbox" in its head; `eval_fixture_is_live_env` fails if the rule regresses.

---

## 2026-10-21 — R45.5: per-vector provenance + query-recorded active fingerprint

* **Decision:** Every chunk vector carries `chunks.embed_fp` (`provider:model:dim`), stamped in `Embedder::sync_vec` (the one hook all embed write paths already call). The query side records its vector's fingerprint in a single-row `embed_state` table before retrieval; the vector legs compare only stored vectors in the same *family* + dim (`embed_fp.rs`). Stale vectors (other family, unstamped) are keyword-only and the scheduler backfill re-embeds them, woken immediately by a `Notify` when a query sees a family change.
* **Context:** R45.4 makes the embedder switchable (built-in candle vs Ollama vs cloud). Today a model change silently compares new query vectors against old chunk vectors (same dim = plausible-looking garbage).
* **Alternatives considered:**
  - *Per-corpus fingerprint only, wipe all vectors on change* — simpler, but destructive (switch back = full re-embed again even for untouched rows) and an in-flight job could write old-model vectors under the new stamp.
  - *Thread the fingerprint through `orchestrate_fusion` / `vector_search` signatures* — exact per call, but touches every caller (`answer.rs`, eval, tests) while R45.2 rewrites the same code; the DB channel keeps the change inside `embedder.rs`/`retrieval.rs`. Residual race: two concurrent queries straddling a settings change inside the 3 s settings cache — accepted.
  - *Provider by URL locality (local vs cloud)* — rejected: a Team image or LAN Ollama (`http://ollama:11434`) would read as "cloud" and force a re-embed of identical weights. The model name identifies the weights; the provider tag is the engine (`ollama` for any HTTP endpoint today, `builtin` from R45.4).
* **Consequences:**
  - Families are an explicit allowlist (`SHARED_FAMILIES`): only Ollama `nomic-embed-text` ↔ built-in nomic v1.5 (probe parity 1.00000). Remote "nomic" endpoints are unproven and re-embed.
  - Migration 22 stamps existing vectors from the DB `embed_model` (or the default); env-only `ANCHOR_EMBED_MODEL` overrides cost one re-embed after upgrade.
  - Backup archives carry `embed_fp`; older archives restore unstamped and are re-embedded.
  - No active fingerprint (fresh DB, hash-vector tests, the golden eval) = no filter, i.e. pre-R45.5 behavior.

## 2026-10-21 — R45.13: vocabulary without proposals (reverses R40.7/R40.8)

* **Decision:** Delete index-time LLM vocabulary (`defined_terms`), the corroboration quarantine and the "Words to confirm" review queue. Vocabulary is a people-owned glossary (typed entries + files a person marks as a glossary) plus one automatic rule: `Full Phrase (FP)` parentheticals that pass an initialism check. All rows are active; there is no proposal status. Local mode may expand unknown acronyms at query time with the answer model (cached, nothing stored); agent mode skips automatic rule rows.
* **Context:** Owner feedback 2026-10-21 ("these proposals are a nightmare"). The owner's DB had 12 LLM proposals (mostly right, still queued for people) and 50 rule rows live with no review, several junk (`psa → card account issuer payment agent`, `cncl → update`, `p2p → payments`) from payment-scheme code tables parsed as definitions. Guessing better would keep the queue; the fix is to stop guessing.
* **Alternatives considered:**
  - *R43.30 as filed: initialism check + queue triage (grouping, bulk reject)* — fixes the worst rule junk but keeps people doing cleanup for model output.
  - *Keep Tier-2 but require two witnesses to even appear* — fewer rows, same model of the feature.
  - *Marking mechanism: a glossary heading convention or a per-source flag* — a heading is guessing again (it is what produced the code-table junk); a source flag is too coarse. A per-item flag on `PATCH /items/:id` mirrors the R35.1 `is_pii` mark people already know.
* **Consequences:**
  - Migration 24 (Rust hook) drops every `llm` row — an R40.8 approval left no trace that tells it from corroboration, and the owner DB had none active — and every `rule` row failing the initialism check. Passing rule rows stay; re-sync re-derives the rest.
  - Correct-but-not-initialism pairs (`rfnd → refund`, `dunning → payment reminder`) now need the glossary. The golden eval seeds `dunning-01` as a fixture glossary row.
  - Glossary-file and rule rows are source-scoped; manual rows are global.
  - Backup now round-trips vocabulary (`vocab` records) and the item mark.

---

## 2026-10-21 — Phase 45 owner decisions (§7.2.1–7.2.3 + acronym default)

* **Decision:** (a) R45.4 ships **nomic v1.5 stored f16 (~274 MB)** from the pinned HF revision (SHA-256s in `docs/probes/2026-10-21-embedding-engine.md`) — no re-embed needed. (b) The built-in embedder is the default on every OS; when there is no Metal, the first index is large and Ollama is reachable, the wizard suggests Ollama (identical vectors). (c) R45.10 demotes the 1,798 pre-R45.8 rule-made `verified` rows to `unverified` and deletes rule rows from non-record items, after a backup, with counts recorded. (d) Local-mode acronym expansion is **opt-in** (`acronym_expansion=true`); the unset default makes zero calls. (e) No glossary re-entry needed now — the owner DB had no approved LLM rows; correct pairs get added by hand via `/glossary` as they come up.
* **Context:** The R45.3 probe holds parity ≥ 0.99995 on every backend, so the size question is download weight vs re-embed cost, not quality; a smaller model would force a full re-embed for weaker search. The 1,798 legacy rows surface as junk in `/decisions` + `/actions` (whole meeting headers stored as "actions"). Expansion-on-by-default spent an answer-model call per new acronym set before search ran.
* **Alternatives considered:**
  - *A smaller embedding model* — rejected: full re-embed + weaker search for a smaller download.
  - *Built-in only, never suggest Ollama* — rejected: a 10k-chunk first index takes ~55 min on Linux CPU vs ~4.5 min with Ollama on Metal.
  - *Demote-only for legacy rows* — rejected: the junk rows stay visible as unverified instead of going away.
  - *Expansion on by default* — rejected: a latency + model-call cost on every acronym-bearing query that the owner did not ask for.
* **Consequences:**
  - R45.4 → R45.6 → R45.7 and R45.10 are unblocked; the R45.3 CI-only legs (MinGW, x86 mkl, Ollama CPU baseline) still need a CI run before the engine decision entry is final.
  - `query_expand::enabled` is true only on an explicit on-value; agent mode still never expands.

---

## 2026-10-22 — R45.16: retire the Python conformance suite (owner decision)

* **Decision:** Delete the CI `backend` job (frozen code testing itself) and the `rust-conformance` pytest leg; port the still-valid HTTP contract into `tests/api_*.rs` (black-box: each test spawns the binary on a free port with a fresh temp DB). Tag `archive/python-final` stays as read-only history — nothing checks it out.
* **Context:** The frozen suite was decaying: 19 deselects pinned removed behaviour (vec0, `/pii/*` auto-detect, who-knows/graph legs, prose-entity extraction), and a full audit showed most of the rest tested Python internals against themselves even in "conformance" mode (queue bounds drove the Python `JobManager`; the diversity-cap tests drove the Python `AnswerEngine`; 16 tests skipped outright vs Rust). Four cloud-gate tests could never run against Rust at all (monkeypatched Python settings). The suite could not be updated without moving the archive tag, so every behaviour change added another deselect instead of a test.
* **Alternatives considered:**
  - *Keep the suite and keep deselecting* — rejected: the deselect list is where contract coverage goes to die; the suite was already down to partial signal.
  - *Update the frozen suite in place (move the tag)* — rejected: editing the archive destroys the "frozen reference" property; the port achieves the same coverage in the shipped language.
  - *Port as in-process router tests (axum oneshot)* — rejected: needs a lib target carved out of the binary; spawning the real binary tests the shipped artifact (migrations, startup sweep, CSRF) with zero production-code changes.
* **Consequences:**
  - `cargo test -p anchorcore` is the only Rust gate (535 unit + 60 contract + 104 golden eval). CI has no Python step left except `version-drift` (which only runs `sync_version.py --check`).
  - Behaviour reframed for current contracts, not copied: dispute exclusion is pinned on `GET /decisions` (entity chunks are never evidence since R33.3); the PII auto-scan tests became a manual-mark round-trip (R35.1); the B15 project-scoping test finally runs (it skipped under the shared-DB harness).
  - Known gaps left open: `answer_reasoning_effort` persists but generation never sends it (found while porting; needs an owner call on whether the setting lives); the `vec_chunks` plain table + sync triggers exist only because the old suite asserted them (removal needs a migration — not done here).

---

## 2026-10-22 — R45.17: the model banner only claims what is actually down

* **Decision:** `/health` gains `embedder.provider` (local/cloud, a pure settings read like `/system/status`), and the banner branches on it plus the mode-aware `answer_ready`: both down / answers only / search only / silent when cloud answers and cloud embeddings cover Ollama. The missing-models line asks for search quality when an answer model is ready. The "classification falls back to simple rules" clause is gone — rules are the default classifier since R45.1.
* **Context:** Owner report: the red "Ollama is offline" banner persisted on an agent/cloud setup where everything worked. The line predates agent mode and the rules default; it never looked at `answer_ready` (which already encodes both).
* **Alternatives considered:**
  - *Frontend-only, branch on `answer_ready` alone* — rejected: with local embeds and Ollama down, search really is keyword-only, but with cloud embeds it is not; the frontend could not tell them apart without the provider field.
  - *Silence the banner whenever answers are ready* — rejected: keyword-only search is a real degradation worth one calm line; per-answer notes alone are easy to miss.
* **Consequences:**
  - `Health.components.embedder` is optional in the UI types (old backends omit it; absent counts as local — the narrow note is the safe side).
  - Adjacent wart left alone: the onboarding wizard's missing-models note ("answers and classification stay basic") is not mode-aware either — first-run only, flagged for the R45.6 wizard finish.

---

## 2026-10-23 — R45.18: ingest is model-free (owner decision)

* **Decision:** Delete LLM entity extraction (`extract_entities=llm`), Q&A distillation (`distill_enabled`) and every classifier-model setting and UI surface. Extraction is the R45.8 record parser only; `extract_entities` keeps `rules` | `off`. Reassess later and add *different* functionality if a need shows up — not resurrect these.
* **Context:** R45.1 made both opt-in, but the Settings tab still showed a full "Classification (extracts entities)" section with a cost note that no longer applied, and neither opt-in had a UI toggle. Value audit: distilled chunks are excluded from the retrieval leaf pool since R33.3, so distillation bought nothing; LLM rows carry no owner/status/due/record id/supersedes, so they fall outside the R45.8/R45.9 record model, while costing one call per 16k window and reintroducing the model-dependent fabrication R45.8 was built to stop.
* **Alternatives considered:**
  - *Hide the classifier fields behind an extraction selector* — rejected: keeps a mode whose output the record listings can't use.
  - *Keep `llm` as an API/env-only opt-in* — rejected: an invisible, untested-in-practice path is noise; decisions in prose are better served by agent mode at question time (no ingest calls, no stale rows).
* **Consequences:**
  - Migration 25 drops `kind='distilled'` chunks; `ingested_items.distill_hashes` stays as an unwritten column. Old archives' distilled rows are skipped on import (their embedding records warn "no local row" and skip).
  - `classifier_api_key` is wiped from the secret store at startup; `PUT /settings` ignores the retired keys (warn-logged). `/system/status` replaces `classifier` with `extraction.mode`; `ollama.classifier_model` and `/health` `components.classifier` are gone.
  - The embedder timeout moved from `classifier_timeout` to `embed_timeout` (default 60s) — a deployment that set `ANCHOR_CLASSIFIER_TIMEOUT` for slow embeds must rename it.

---

## 2026-10-23 — Phase 46: flows and images stay model-free; unplaced images are attributed by people (owner decision)

* **Decision:** Visual content is indexed without any model on the ingest path (R45.18 holds). Flows/diagrams are structured data and are converted deterministically to text (`diagram.rs` normal form: frames → headings, edges → `A → B: label`). Images are stored as assets found through their context (alt, caption, hosting section, documents that link them) and are *seen* by the agent at question time (MCP image content block / REST `GET /assets/:uid`). An image with no context goes to a people-owned "Unplaced images" view where a person writes a caption and/or attaches it to an item, section or entity; that caption is the only text the image ever gets.
* **Context:** Owner question 2026-10-23: the engine cannot surface pictures or whiteboard flows (Miro). Most flow tools already expose a node/edge graph, so the gap is parsers, not understanding. For bare images the only model-free signal is context; where there is none, a person is the attribution source — the same rule as the glossary, the PII mark and `verified`.
* **Alternatives considered:**
  - *OCR at ingest (Tesseract)* — rejected for now (owner): a model on the ingest path in all but name, adds a native dependency, and mostly helps scanned PDFs, which are a separate problem.
  - *Vision captioning at ingest (local VLM)* — rejected: stores model-written text as evidence, the fabrication class R45.8/R45.18 removed.
  - *Agent-suggested captions in the Unplaced view* — rejected (owner, 2026-10-23): reintroduces a proposal/review queue (R45.13 rule). Reopening needs a new owner decision.
* **Storage (owner, 2026-10-23):** asset bytes are always stored in the DB, for every source kind including folders — never path references or fetch-on-demand. Export/backup must restore every citable image even when the original source is gone or offline. Assets dedupe by content hash. Size limits are set from measurement (R46.14): a configurable per-asset cap (`asset_max_bytes`, provisional default 20 MB) keeps over-cap assets as metadata-only rows with a warning instead of failing the sync.
* **Consequences:** Unattributed images are not searchable until a person places them (a deliberate gap, visible as a count). PDF images are out of scope. DB and backup size grow with image-heavy sources — surfaced in System status and before export (R46.14).

---

## 2026-10-23 — R30.1: one active sync/reclassify per source; explicit duplicates 409, background coalesces

* **Decision:** Sync and reclassify share one active slot per source. An explicit `POST /sources/{id}/sync` or `/reclassify` while a sync-family job is pending/running returns 409 `{"detail": "…already active…(job N)", "job_id": N}` with no new row; watcher and scheduler triggers coalesce silently onto the live job. The guard lives in `JobManager::create_job`, checked in the same `BEGIN IMMEDIATE` transaction as the insert under `PROMOTE_LOCK`. A reclassify requested while a sync runs 409s like any other duplicate — it is not queued behind the sync.
* **Context:** Owner screenshot 2026-10-23: jobs #73/#74 both `running` at 3649/4833 with #75 `pending`, created 3 s after #74 — two syncs duplicating work on the same data plus a queued third. Same class as the R30.1 repro (sync+reclassify 4 s apart via the poll-lagged UI badge) and the M4 repro (scheduler's 60-minute folder scan auto-creating #7 over running #6). Audit H4 files the same gap.
* **Alternatives considered:**
  - *Queue the reclassify to start after the sync (the row's recommendation)* — rejected: a queued-behind row violates the at-most-one-pending/running invariant the 409 pins, and needs source-aware promotion (today `maybe_promote` takes the oldest pending globally, so the queued reclassify could start on another slot while the sync still runs). The UI already serializes the two buttons; a direct API caller gets the live `job_id` and retries after it finishes.
  - *Coalesce explicit POSTs too (202 with the live job)* — rejected: the caller asked for a new run (possibly `force`), and a 202 would lie about starting one; 409 with the live id is honest and the UI already renders `detail`.
  - *Guard in each caller instead of `create_job`* — rejected: three trigger paths (manual, scheduler, watcher) would each need the check-then-insert race closed; one guarded constructor closes it once.
* **Consequences:**
  - `create_job` returns `Result<i64, CreateJobError>` (`Conflict(active_id)` vs `Db`); `embed` jobs are outside the sync family and unguarded. Pre-existing duplicate rows (created before this change) drain naturally; the guard only blocks new ones.
  - A job stuck `running` with no live task (a pipeline panic — the R43.11 guard is sequenced after this) now blocks that source's syncs until it is cancelled or the process restarts (both paths already handle orphans). Cancel stays the escape hatch.
  - The `folder_scan_minutes` Settings exposure the row allowed as scope was skipped: with duplicates impossible, a long first ingest no longer needs the interval workaround.

---

## 2026-10-23 — R46.7: how image assets are stored, linked and archived

* **Decision:** Image bytes live in `asset_blobs` (sha256 primary key); `assets` rows reference a blob by hash, so identical images are one blob by construction and two triggers delete a blob once nothing references it (FK cascades included). Two origins: `embed` (a reference in a markdown/HTML document, on its hosting chunk) and `file` (an image file item of a folder source). Embeds resolve folder-relative paths inside the canonical source root only, plus `data:` URIs for any source; remote URLs are not fetched. Backups carry blobs as stored `assets/<sha256>` zip entries, verified against their name on import and read before `data.jsonl`.
* **Context:** Owner storage decision (Phase 46 entry above): bytes always in the DB, export restores every image without the source. R46.7 had to choose the blob layout, the archive encoding and the resolution boundary.
* **Alternatives considered:**
  - *Bytes inline on each asset row* — rejected: the DoD's dedupe ("one blob, many asset rows") would need app-side reference counting; a hash-keyed table plus GC triggers makes it structural and survives cascades.
  - *Base64 blob records inside `data.jsonl`* — rejected: +33% size, counts against the 4 GB `data.jsonl` cap, and makes every image a multi-MB JSON line; separate stored entries add no compression cost (images are already compressed) and let R46.14 size an export from the blob table alone.
  - *Fetch remote image URLs at sync* — rejected: network calls to arbitrary hosts on the ingest path (SSRF surface, offline-first breakage); their alt text is already searchable chunk text. Revisit per connector (Drive/Miro rows fetch through their own authenticated APIs).
  - *Separate `title` and `caption` columns* — merged into `caption`: markdown has one quoted slot, and HTML `title`/`<figcaption>` would otherwise need an out-of-band channel through tag stripping.
* **Consequences:**
  - HTML `<img>` now survives extraction as `![alt](src "caption")` text (the figcaption text also stays in the prose); HTML `data:` images contribute only their alt text, so inlined base64 never enters chunks — they get no asset until an HTML path that sees raw markup exists.
  - A file over `asset_max_bytes` keeps its row (`bytes_hash` NULL, real `size`) and warns once per transition to over-cap; image file items over the cap hash as `oversize:<len>:<mtime>` so their bytes are never read.
  - `.svg` files are both a text item (R46.2) and a `file` asset. Repos with icon sets therefore add blobs (small) and empty-text items; R46.14 reports the cost.

---

## 2026-10-23 — R46.9: asset bytes are served at `/media/:uid`, not `/assets/:uid`

* **Decision:** `GET /media/:uid` serves an asset's bytes from its DB blob, with the `/qa/search` label gate (403 `refusal: "gated_provider"` for a sensitive/pii source under an untrusted answer provider), a sandboxing CSP and `nosniff`. Later Phase 46 rows (R46.10 skill docs, R46.12 Unplaced listing/PATCH) use the same `/media` prefix.
* **Context:** The Phase 46 draft named `/assets/:uid`. `/assets/*` is Vite's bundle directory, embedded in the binary and allow-listed as public in `auth::is_public_path` so the login page loads with auth on.
* **Alternatives considered:**
  - *Keep `/assets/:uid` and special-case the auth allowlist* — rejected: the route would also match the UI's own `/assets/index-*.js` requests (explicit routes beat the frontend fallback), breaking the app, and the allowlist would need hash-shape heuristics to tell the two apart.
  - *Rename Vite's output dir* — rejected: touches the build, the embedded-UI serving and every cached client for a naming preference.
* **Consequences:** Image responses are inert even for user SVG (CSP `sandbox`, `default-src 'none'`). Over-cap assets answer 404 with a distinct detail; their metadata stays visible on `GET /items/:id` and in hit `images` (with `url: null`).

---

## 2026-10-23 — R46.5: Miro skips unchanged boards via stamps in `last_sync_cursor`

* **Decision:** The Miro connector compares each board's `modifiedAt` with a stamp `{modified_at, hash}` stored per board in `sources.last_sync_cursor` (`{"miro": {...}}`). A board is skipped (items + connectors never fetched) only when the stamp matches and the stored item has `content_hash = processed_hash = stamp.hash`; the pipeline then hands the stored doc back so upsert hash-skips and purge keeps it.
* **Context:** API connectors fetch everything and hash-skip; for boards with thousands of items the item fetch is the cost (and the rate limit). `last_sync_cursor` was written at create and never read by any sync path.
* **Alternatives considered:**
  - *Hash-skip only (fetch every board every sync)* — rejected: burns Miro's rate-limit budget on unchanged boards, which is the common case.
  - *Stamp on `ingested_items.updated_at`* — rejected: `upsert_doc` is shared by every connector and owns those columns; a connector-private cursor keeps the change local.
  - *Stamp by `modifiedAt` alone* — rejected: a sync that fetched a board but failed before storing it would leave a matching stamp over stale text forever; binding the stamp to the processed content hash makes that case refetch.
* **Consequences:** Deleting a board purges its item only when at least one other board survives (the shared empty-fetch guard purges nothing on an empty result). Citations name the board; per-item deep links wait for a chunk→item map.


---

## 2026-10-23 — R46.15: Drive Office images resolve from the whole downloaded package

* **Decision:** `gdrive.rs` keeps the downloaded docx/pptx/ODF bytes on `IngestionDoc.package` (an `Arc<Vec<u8>>`) only when the extracted text carries `embedded:` markers; `assets::store_embedded` reads parts from it, falling back to the file on disk for folder sources. `resolve_embedded` is unchanged, so both sources apply the same size cap, sniffing and decorative-image skip.
* **Context:** Drive had no file to re-read after extraction, so Drive Office image markers stayed alt-only text. The backlog row allowed either the package or the pre-extracted parts.
* **Alternatives considered:**
  - *Extract the referenced parts in the connector* — rejected: the connector has no settings, so it can't apply `asset_max_bytes` and would read every image in full anyway. It would also duplicate `resolve_embedded`'s zip handling. Images are most of a deck's bytes, so the memory saved is small.
  - *Re-download at the asset pass* — rejected: a second network round-trip per document and a second token use, for bytes the sync already had.
* **Consequences:** A Drive fetch holds image-bearing Office packages in memory until their docs are stored (the fetch collects all docs before classifying). Fine at Drive folder sizes today. If R46.14's measurements show large decks, stream fetch → store per file instead of trimming the package.

---

## 2026-10-23 — R46.12: a person's caption is a derived chunk; targets stay in the image's source

* **Decision:** `PATCH /media/:uid` stores the caption and target on the asset (`manual_caption`, `manual_target`) and derives one `kind='caption'` chunk from them (uid `caption:<asset uid>`), placed on the target (an item, a section of one, or an entity's item) or on the image's own item. Targets must be in the image's own source. They are stored as item uid + section path / entity uid. Caption chunks join the retrieval leaf pool and cite as `kind: "caption"` with path `… > caption by a person`.
* **Context:** Owner decision (b) makes captions people-only. The row asks that a caption answer questions, survive re-sync and backup, and take its target's label. A document re-sync deletes and rebuilds the item's chunks and sections with new ids.
* **Alternatives considered:**
  - *Caption as an asset field only, matched at query time* — rejected: no FTS/vector leg reads `assets`, so the caption could not answer a question without a fourth retrieval leg.
  - *Store target ids* — rejected: section ids change on every re-sync and on import. Uids + section path re-resolve after both.
  - *Allow cross-source targets* — rejected for now: the caption would take the target's label while the bytes keep the image source's label. One gate would then say "sensitive" and the other "public" for the same picture. Same-source keeps one label. Widening it needs a rule for which label wins.
  - *Rebuild caption chunks on every store (delete them with the document's chunks)* — rejected: an unchanged caption would lose its vector on every sync. The chunk is kept and only re-pointed. Content or path changes clear the vector.
* **Consequences:** `ignore` deletes the chunk but keeps the asset and the person's text, and un-ignoring restores both. Clearing caption and target hands attribution back to sync (`embedded`, or `none`/`referenced` for files). Deleting a target item cascades its caption chunk away; the people fields stay on the asset, and the next PATCH re-materializes it on the image's own item.

---

## 2026-10-23 — R46.16: PDF pictures are extracted, never rendered (owner decision)

* **Decision:** A PDF's image XObjects become embed assets. The owner's rule: if a PDF has a picture, extract it; if it doesn't, ignore it. `pdf_images.rs` walks each page's content stream for drawn images (`Do`, through Form XObjects up to depth 4). It does not trust the resource dictionary, because generators share one resource dict across pages. Extraction adds one `![](embedded:pdf:<obj>-<gen>)` line to the end of the image's page block. `assets::store_embedded` resolves the marker from the PDF on disk (folder) or from the downloaded package (Drive). JPEG (`DCTDecode` only) passes through unchanged. 8-bit Gray/RGB samples are re-encoded as PNG, with an 8-bit `SMask` used as alpha. JPX/JBIG2/CCITT/CMYK/Indexed/stencil images and other bit depths are skipped. The context is the page (`p. N — nearby words`). An image on a page with no words of its own goes to Unplaced images (`attribution='none'`).
* **Context:** PDFs were text-only. A drawn picture in a PDF is the same kind of thing as one in a docx (R46.8). Most "save as PDF" charts are vector drawings with no image object, though.
* **Alternatives considered:**
  - *Rasterize pages (pdfium/mupdf)* — rejected: it needs a large native dependency on all three platforms, and every vector page would become an image with no context. The owner's rule is extract-or-ignore.
  - *Attribute images from the page resource dict* — rejected: shared resources would attach every image to every page.
  - *Decode CMYK/Indexed/JPX* — deferred: it needs color management or codecs. Skipped images stay out until real corpora show they matter (R46.14 measurement).
* **Consequences:** `lopdf` + `png` are now direct deps (the same versions `pdf-extract` already pulls in). Overlapping prose chunks re-read the same marker, so each ref is capped at its count in the item text. Clearing a person's caption gives back the page-derived split (`embedded` or `none`) through `assets::embed_sync_attribution`. Captions on PDF images sit at `p. N > caption by a person`.

---

## 2026-10-26 — Backlog review: close what the model-free direction made moot

* **Decision:** every open row in `rust/BACKLOG.md` and `docs/product-plan.md` was checked against the code. Closed as out of direction: R45.11 (LLM enrichment — R45.18 deleted `extract_entities=llm`), R39.11 (cross-encoder reranker — a model would score chunks again; in agent mode the agent reranks), B16 (who-knows ranking — leg deleted in R45.9; ownership is `GET /actions`). Closed as covered: R36.4 (uniform `embed_input` + extractive section summaries), R36.13 (glossary, R45.13), R36.12 (no graph-evidence consumer after R45.9), R33.7 (contract ported to `tests/api_gates.rs` by R45.16), R36.9 (into R43.12), R42.1 (into R46.14). Rescoped to what remains after R45.18: R30.2 (classify concurrency → embed batching, the measured bottleneck), R30.3, R30.5, R30.6 (answer path only), R35.2 (embed + answer, no classify), R43.17, R43.20, R43.21. R45.10 raised to P1 as the data half of R45.18.
* **Context:** R45.18 removed every model from the ingest path, but rows written before it still assumed classify/distill calls, LLM summaries or graph evidence. Embeddings are not classification, so the built-in embedder track (R45.3/R45.4) and deterministic structure work (Phase 36) stay. Shipped rows and fully shipped phases moved verbatim to `BACKLOG-archive.md`; reasons per row are in BACKLOG.md → Closed 2026-10-26.

---

## 2026-10-26 — R45.10: legacy entities are told apart by the parser marker, backed up first

* **Decision:** migration 27 treats every entity whose `reasoning` is not `record parser` as legacy (LLM extractor or the pre-R45.8 rule classifier). Since R45.8 (dd24bb3, 2026-10-21) the parser is the only entity writer, and it stores rows `unverified` with that marker. Following the owner's §7.2.3 (a)+(b): legacy rows on items outside the `record` bucket are deleted, including unbucketed `doc_type=''` items, and their entity chunks cascade with them. Remaining legacy `verified` rows become `unverified`. Parser rows are never touched, so a `verified` parser row stays verified because only a person can have set it. Before the migration runs, `init_db` writes `exports/safety-backup-pre-r45.10-<utc>.db` (`VACUUM INTO`, never swept). If that backup fails, the boot fails instead of changing data that has no backup. Counts go to `system_events` (`component='migration'`).
* **Context:** there is no provenance column and no audit trail for person verification (`PATCH /entities` does not touch `updated_at`). On the owner's DB, all 1794 verified rows predate R45.8 and none carries the marker. A dry run on a copy: 1799 legacy → 1611 deleted, 185 demoted, 188 left, 91 entity chunks cascaded, 0.3 s.
* **Alternatives considered:**
  - *Cut by `created_at` at the R45.8 date* — rejected: it depends on the clock, and the marker is exact.
  - *Demote only, leave deletion to a force reclassify* — rejected: owner chose (b). A reclassify rebuilds record rows from structure anyway.
  - *Re-bucket unbucketed items inside the migration* — rejected: bucketing needs the pipeline's text path. Their legacy rows go, and a Reclassify brings back whatever their structure supports.
  - *Proceed without a backup when `VACUUM INTO` fails* — rejected: the row's DoD is "backup first".
* **Consequences:** upgraded installs lose most model-era entities on first boot. Release notes must say "run Reclassify on each source to rebuild decisions/actions from structure" and point to the backup file. A person who verified a legacy row loses that mark (accepted in §7.2.3).

---

## 2026-10-26 — R30.2: embed concurrency kept, but the bottleneck is model compute

* **Decision:** `Embedder::embed` keeps up to `ANCHOR_EMBED_CONCURRENCY` batches in flight (default 2, cap 8), with one pooled HTTP client and results in input order. Cross-document batching is not built. Re-sync cost is cut with exact-match vector reuse instead (R43.12).
* **Context:** R30.7 read "82 s of a 91 s file were 263 back-to-back embed batches" as a sequencing problem. Measured back to back against local Ollama (`nomic-embed-text`, Metal), throughput was the same at concurrency 1, 2 and 4: ~35 chunks/s, which matches the 2026-10-21 probe's Ollama rate. Ollama computes embeddings one request at a time, so the wait is model compute. 600 files: 417 s → 419 s; a 4504-chunk file: 240 s → 227 s.
* **Alternatives considered:**
  - *Batch across documents* — rejected: it can't raise a compute-bound rate, and it would break the per-document phase order (embed is the last phase of each file's finalize).
  - *Drop the concurrency knob* — rejected: remote OpenAI-compatible endpoints are latency-bound, and there overlap is a real gain. Default 2 costs nothing locally.
* **Consequences:** first-index speed with local Ollama is set by the model. The built-in embedder (R45.4) is the next lever there (Metal f16 measured faster per query). For re-syncs, R43.12 removes the embed cost of unchanged chunks; the tags phase is now the largest re-sync cost (R30.5).

---

## 2026-10-26 — R47.0: bug reports are a redacted endpoint over retained query metadata

* **Decision:** `GET /system/report` serves a redacted bundle for one of the last 20 `/qa`/`/qa/search` calls: query text, per-hit legs/ranks/scores, index counts, versions, embedding family — no document text, titles, paths, settings, or secrets (pseudonyms + hashes; the structs lack the fields, and `tests/api_report.rs` pins the exact key set). Retention hooks sit in the `routes.rs` handlers over a process-global metadata-only ring, so UI, REST-agent and MCP-over-HTTP traffic is all covered with no `answer.rs` signature churn. The Ask tab gets a Report button; the `anchorcore-search` skill documents the one-call agent flow.
* **Context:** owner asked for reporting "both Agents and People can use" after a retrieval investigation that needed three hand-rolled evidence captures. Rejected a new skill as too heavy; GitHub-issue filing stays a maybe (the bundle is paste-ready JSON).
* **Alternatives considered:**
  - *Skill-only collection (agent gathers via existing endpoints)* — rejected: redaction by prompt compliance instead of by construction, and a repeat of the bespoke-capture dance every time.
  - *In-app-only download* — rejected: leaves REST/MCP agents without the flow the owner explicitly asked for.
  - *Re-run the query at report time* — rejected: ranks drift as the index changes and it costs an embedding call; retained diagnostics describe what the user actually saw.
  - *AppState-held ring* — rejected: 11 constructor edits for no gain; black-box tests are separate processes so the global ring never leaks between tests (unit tests must not assert on it).
* **Consequences:** last-20 queries' metadata lives in process memory (~10 KB each worst case). Any future serialized field trips the redaction test first.

---

## 2026-10-26 — R47.1: rich keyword evidence pierces the authority-tier partition

* **Decision:** FTS legs flag `Hit.rich_match` when a row matches ≥3 distinct surface forms (`ANCHOR_PIERCE_MIN_TERMS`) with BM25 magnitude ≥5.0 (`ANCHOR_PIERCE_MIN_BM25`); the tier key becomes (env, pierced, tier), so rich evidence sorts on score order within its env class while thin matches stay tiered. Either knob negative disables the pierce. Exposed as `rich_match` on search hits/citations and `pierced` in the R47.0 report bundle.
* **Context:** two PDFs vanished from a 5k-item index for queries whose terms they contain (even the PDFs' own title words). Root cause, proven with captured leg ranks: term scatter gives single discounted floor legs; the (env, tier) partition then sorts all Code/Config above Prose before the cut — the global keyword-1/2 chunks were cut too. Repair/extraction was exonerated first (intact text, `suspect` flags are domain-vocabulary false positives). Per-tier minimums were rejected: the cut already contained Prose — the PDFs lost *within* Prose, so only evidence-ordered rescue reaches them.
* **Alternatives considered:**
  - *FTS `matchinfo` per-phrase bitsets* — rejected: FTS5 has no `matchinfo` (FTS3/4-only); discovered when the leg returned zero hits. `highlight()` spans give the same FTS-accurate matched set with no stemmer dependency.
  - *Deeper slots* — rejected as the primary fix: fixed-count rescues don't scale (2 slots vs millions at org scale) and can't reach within-Prose burial while top floor ranks consume them.
  - *Score multipliers for Prose* — rejected: ordering-only rule (R32.7); partitions reorder, multipliers don't compose with RRF.
* **Consequences:** tier semantics narrow to thin matches; rich matches everywhere behave as if tiers were off. Small corpora self-disable (magnitudes can't clear the bar where burial is impossible). Calibration (thin-max ≈ 2.3, rich-min ≈ 7.4) is pinned by the burial repro and the flags-01 guard tests; golden eval stays 1.00.

---

## 2026-10-26 — R45.4: built-in embedder ships; `auto` prefers a serving Ollama (owner decision)

* **Decision:** `embed_provider` = `auto` (default) | `builtin` | `ollama`. `auto` uses the HTTP embedder when it serves: an explicitly configured cloud `embed_base_url`, or a local Ollama that has the embed model (checked against `/api/tags`, cached 60 s, re-checked before each embed call and on `GET /system/embed-model`). It falls back to the built-in candle engine (nomic-embed-text v1.5, f16, Metal on macOS, pure CPU elsewhere) when its model is ready and nothing better serves. The weights are fetched only on an explicit action (wizard/Settings button, `POST /system/embed-model/download`, `anchorcore embed-model fetch`). They are pinned to HF revision `e9b6763…` with per-file SHA-256, the download is resumable, and dropped-in files are adopted offline. The f32 download is converted once to f16 (273 MB) and the f32 file deleted. The Team image prefetches the weights at build time (`ANCHOR_MODELS_DIR=/opt/anchorcore/models`).
* **Context:** the 2026-10-21 decision (§7.2.2) was "built-in first", based on a probe that put Metal f16 within ~1.4× of Ollama. In-app measurements on the owner's M2 Pro (release build, quiet machine, 600-file synthetic corpus, 7507 chunks): built-in 6.8 chunks/s (1101 s of embedding) vs Ollama ~34 chunks/s (222 s). A single query takes 21 ms built-in. Larger batches help the built-in engine (64 texts in 450 ms in isolation), but the pipeline embeds per document (~12 chunks per call). Parity with Ollama, live: cosine 0.999999 (Metal f16), so the vectors stay one family and switching re-embeds nothing. Asked with these numbers, the owner chose "prefer Ollama if serving".
* **Alternatives considered:**
  - *Built-in whenever downloaded (the 2026-10-21 plan)* — rejected by the owner: Ollama users who downloaded the model would index ~5× slower.
  - *Built-in, with a UI hint to switch on big indexes* — rejected: it puts the choice on the person for something the app can detect.
  - *Cross-document batching for the built-in engine only* — deferred: it would change the per-document phase order (see R30.2). It is the lever if built-in bulk speed matters on machines without Ollama.
* **Consequences:** a machine without Ollama gets hybrid retrieval after one ~550 MB download (verified end to end: Ollama unreachable → `retrieval: hybrid`, backend `metal`). Ollama users see no change. `ollama.rs` readiness no longer asks Ollama for the embed model while the built-in engine is active. The suite needs no weights. Real-model tests are opt-in via `ANCHOR_TEST_BUILTIN_MODEL_DIR`. Windows MinGW and x86 Linux builds are proven only by CI (R45.3 legs) before this merges to `main`.

---

## 2026-10-27 — R36.2: config rule-level sections + per-item chunker generation with boot-time re-chunk

* **Decision:** YAML/JSON/JSONC/TOML take a new splitter (`config_chunks.rs`). Sections are top-level keys (TOML tables); a top-level value that is a list of mappings becomes one section per entry under the key ("list is the main content" = the key's direct children are all entries, 2..=500 of them, at least half mappings; same rule for YAML/JSON root lists and TOML `[[name]]` runs). Consecutive top-level scalar keys share one section. Entry titles promote identifying scalars (`id`/`name`/`tenant`/`settle_code`/`item_type`/`*_id`/`*_code`/`*_type`/…, camel/kebab normalised, ≤3 pairs, values ≤32 chars) with a first-short-scalar fallback (booleans skipped) and `#N` last; keyed sections append identifying children in parentheses; pairs also go to `sections.meta.promoted`. Chunks are exact source line ranges grouping keys into 500–800 chars (a run under 500 may grow to the 1600 hard cap; a key over the cap splits along its sub-keys, else by lines), with their own line ranges. Every chunk write stamps `ingested_items.chunker_version` (migration 28, existing rows start at 1; config = 2), and `db::init_db` (plus backup import) re-chunks items whose generation is older from stored text, one transaction per item, vectors carried over by the `(kind, path, content)` key, the rest left to the embedding backfill.
* **Context:** R33.4 every-key sections turned a rule list into hundreds of one-line chunks (`routes > settle_code`) that matched the identifier but never carried the answer. Phase 36 sign-off (a) chose rule-level sections.
* **Alternatives considered:**
  - *Invalidate `processed_hash`/fingerprints in the migration and let the next sync re-chunk* — rejected: mixed granularity until each source syncs (folder poll is 60 min, API sources refetch over the network).
  - *Re-chunk inside a migration hook* — rejected: `chunks.kind/path/section_id` may only exist after `ensure_columns`, and `run_migrations` drops the `vec_chunks` sync triggers for the run, so deletes there would orphan vector rows.
  - *Main-content lists at any depth* — rejected for now: depth-1 (and root) is deterministic and covers the routing shape; deeper lists still get key-grouped chunks.
  - *Promote long ids (UUID tenants)* — rejected: 36-char opaque ids crowd the breadcrumb; they stay in content (the tenant line sits inside its rule's section).
* **Consequences:** a future splitter change bumps its family in `chunking::chunker_version` and gets the same upgrade for free. Items that host images are not re-chunked at boot (asset placement needs the package) — no such family is versioned yet. Config chunk line spans exist now, so R36.18b shrinks to byte offsets + backfill.

---

## 2026-10-26 — R30.6: local Ollama answers use native `/api/chat` with `num_ctx` pinned

* **Decision:** every local-mode chat call (generation, follow-up rewrite, acronym expansion, the settings "test answer model" ping) goes through `llm_chat.rs`. When the answer base URL is a loopback Ollama — port 11434, or the same root as `ollama_base_url` — the call is `POST {root}/api/chat` with `stream: false` and `options: {num_ctx: 8192, temperature, num_predict}`; `ANCHOR_OLLAMA_NUM_CTX` overrides the 8192. A 404/405 from the native endpoint (some other server on that port) retries once as OpenAI-compatible. Cloud, BYO OpenAI-compatible and other local servers keep the exact `{base}/chat/completions` payload.
* **Context:** Ollama ≥ 0.15.5 sizes the context from free memory (32k tokens on a 32 GB Mac → a 3B model at ~12 GB) and its OpenAI-compatible endpoint ignores context options (ollama/ollama#18229). 8192 covers the 6000-token generation budget (R43.7) plus prompt and answer. Verified live on Ollama 0.32.5 with `llama3.2:3b`: the OpenAI endpoint loaded it at Ollama's auto size (4096 on that machine), the app's answer and settings ping load it at 8192.
* **Alternatives considered:**
  - *Document `OLLAMA_CONTEXT_LENGTH` as the supported control* — rejected: it is a server-wide env the app cannot set (Ollama is often launched by its own app or launchd), it applies to every model and client on the machine, and an unset value silently keeps the memory-sized default. The code fix is ~100 lines in one module.
  - *Send `options.num_ctx` on `/v1/chat/completions`* — rejected: ignored by Ollama (the issue above).
  - *Detect Ollama by probing `/api/version`* — rejected: an extra round-trip per call; the port/base-root check plus the 404 fallback covers misdetection.
* **Consequences:** the settings ping and the answer path load the model with the same context, so the first answer no longer forces a reload. Ollama users who want a bigger window set `ANCHOR_OLLAMA_NUM_CTX`. Streaming stays a follow-up.

---

## 2026-10-26 — R30.4: Stop drops the in-flight provider call; partial ingestion is kept

* **Decision:** a running job's provider calls (the per-document embed in `embed_chunks` and the `embed` job batches) are raced against its cancel (`pipeline::until_cancelled`, polling an in-process flag every 100 ms and the `jobs` row every second). On cancel the future is dropped: an HTTP embed request is aborted, and the built-in engine finishes at most its current 64-text group (`BUILTIN_GROUP`). The flag lives in `jobs.rs`, keyed by `(data_dir, job id)` and set only by the cancel handler; the `jobs` row stays the authority. `JobManager::cancel` is one conditional `UPDATE … WHERE status IN ('pending','running')`, so a second cancel, or one racing the job's own finish, is 409. Every terminal write in `finish_job` / `complete` is conditional on `status <> 'cancelled'`. Documents finished before the Stop are kept, and the interrupted document is not marked processed.
* **Context:** the old checkpoints sat only between documents and phases, so a Stop waited out the whole current document (~15 s measured) and a hung call blocked it until `embed_timeout`. Alternatives: (a) finer checkpoints only (cancel still waits for one full multi-batch embed call); (b) a per-job `CancellationToken` threaded through `ClassifyCtx` (cleaner, but it touches every constructor in `pipeline.rs` while other work edits that file; the flag-plus-row poll gives the same latency); (c) rolling back the partial ingest (expensive, and it throws away finished work that the next sync would only redo). Measured Stop→stopped: 24–98 ms on a 500-file + 200-file synthetic corpus with local Ollama (debug build), and 83 ms with a hung fake provider (`tests/api_jobs.rs`).

---

## 2026-10-27 — R30.10: a single-word tag is its own identity

* **Decision:** after the exact-name lookup misses, `tags::ensure_tag_cached` creates a new tag for any name that is one token under the tag-vector tokenizer (`[\p{L}\p{N}]+`; a name with no token counts as one). Cosine reuse at `tag_reuse_threshold` stays only for multi-word names. The stored vector (`deterministic_embed`, 768 dims) is unchanged, so there is no migration: old tags still answer exact lookups and multi-word cosine reuse.
* **Context:** `propose_tags` only proposes single words (`[\p{L}\p{N}]{4,}`), and a single word's hash vector is one-hot in one of 768 buckets. So the cosine between two single words is 1.0 (same bucket) or 0, and every cosine "reuse" was a hash collision or a test-only probe synonym (`database` → `storage`). The table stopped growing near 768 tags, and the retrieval tag leg (`matching_tag_sections`, query token = tag name) then missed the merged words and pulled the wrong sections for the surviving names. Measured on the 600-file synthetic corpus (release, embeddings off): flat vocabulary 768 → 17377 tags, Zipf vocabulary 471 → 715. The tags phase got no slower: full sync 4.5 s → 3.7 s (flat) and 3.2 s → 2.7 s (Zipf), and the vector cache is never loaded (`scans` 601 → 0).
* **Alternatives considered:**
  - *Wider vectors (e.g. 2^20 buckets)* — rejected: collisions get rarer but never stop, the vector format changes (a migration that re-embeds every tag), and for one-hot vectors cosine is still just "same bucket", which an exact name check already answers.
  - *Collision check (reuse only when the names also match)* — rejected: for single words that is exactly exact-name identity, with extra work.
  - *Drop cosine reuse and the `tag_reuse_threshold` setting* — deferred to the owner: ingest never proposes a multi-word name, so the threshold no longer changes anything ingest does. Removing a user-visible setting is a separate choice.
* **Consequences:** tag count grows with the vocabulary. On the flat corpus that is ~29 tags per document. Tags with zero uses are still pruned when items go away (`pipeline.rs`). Links written under the old scheme (word B's chunks under colliding tag A) remain until that item is re-processed (a content change or a reclassify), because tags can't be re-proposed in SQL. New words get their own tags from the first sync after upgrade.

---

## 2026-10-27 — R35.2: the manual item PII mark gates cloud flows like a `sensitive` label

* **Decision:** `ingested_items.is_pii` is enforced with the label gate's exact rules toward an untrusted provider. One helper, `common::cloud_gate_reason(label, item_pii)`, decides for ask/agent compose, `/qa/search` (ranked and exact, incl. `public_only`), `/qa/public`, `/decisions`, `/actions` and `/media/:uid`. A source label names the cause first; a marked item on an ordinary or public source is blocked as `item_pii`, which shows up in `blocked`, the `answer gate` audit event's `labels=[…]` detail, and `refusal: gated_provider` when nothing survives. Hits resolve the mark through their item (the hit's `item_id`, else the chunk's item, entity chunks via their entity). Embedding: ingest `embed_chunks` gates the whole item, the re-embed job (`fetch_embed_batch`, backup import) and the startup/15-min backfill (`backfill_select_sql`) leave marked items' chunks out of untrusted batches. Ingest and the re-embed job record one `embed` warning event per skip. The backfill only logs, as the label gate does, because a new event every 15 minutes would bury real errors. Follow-up rewrite and acronym expansion already run only for a trusted answer provider, and they read the person's own history, not item content, so they are unchanged.
* **Context:** R35.1 made PII a person's whole-document mark and left enforcement to this row. Until now the marks were advisory.
* **Alternatives considered:**
  - *Gating marked items out of `public_only`/`/qa/public` whatever the provider (as a `sensitive` source is)* — not done. The row limits enforcement to untrusted cloud flows, and a public source with a trusted provider keeps serving a marked doc. **Owner question:** should a PII mark also drop the doc from every public scope? That is a one-line scope filter in `answer::project_scope` and `records_api::resolve_scope`.
  - *A separate item gate in each path* — rejected: `/decisions` and `/actions` must mirror search, so the shared helper keeps them aligned.
  - *Clearing stored vectors when an item is marked* — not done: vectors already sent to a cloud embedder cannot be recalled, and local vectors leak nothing. The setup skill tells people to mark before connecting a cloud embedder.
* **Consequences:** black-box contract in `tests/api_pii_marks.rs` (untrusted withheld on every read path + three embed paths; localhost and `ANCHOR_CLOUD_TRUST=1` flow; unmarked unchanged; clearing the mark lifts the gate). The mock untrusted embedder is reached as `http://0.0.0.0:<port>`: the trust check blesses only `localhost`/`127.0.0.1`/`::1`.

---

## 2026-10-27 — R36.15: agent read APIs (section text, line windows, prefix errors) over REST and MCP

* **Decision:** section text is `GET /nodes/:id?include=text`, the only section-text path. It returns the subtree's leaf chunks (document + caption kinds, document order, `\n\n`-joined, the same rule as the search parent-return) with `text_truncated`, `text_chars` and `text_chunk_count`, capped by `max_chars` (default 32000). Line windows are `GET /items/:id?lines=a-b|a|a-`: 1-based and inclusive over the stored item text. An end past EOF clamps (`sed -n`), a start past EOF or a malformed range is a 400 naming the forms or the line count. `path_prefix` on `GET /items` and `POST /qa/search` (both modes) rejects input that can never match a stored path: `*`/`?`, `..` segments, a leading `./`/`~`, URL schemes, drive-relative `C:x`, control characters. The 400 names the expected form. The owner's deferred MCP call is implemented now: `list_items`, `get_item` (`lines`; MCP drops the `text` alias and cuts content at 32000 chars with `content_truncated` + `total_chars`) and `get_outline` (nodes folded in through `node_id` + `include` + `max_chars`). Each forwards only its REST keys. The CI/release handshake lists gain the three names (the R20.5 bump).
* **Scope and gate:** a shared `read_gate.rs`. `public_only=true` on `/items`, `/items/:id`, `/items/:id/outline` and `/nodes/:id` serves only `public` sources; anything else is 404, so existence is not confirmed. The new byte forms (line windows, node `text`) refuse `sensitive`/`pii` sources under an untrusted answer provider with 403 `refusal: "gated_provider"` plus a gate event, mirroring `/media`. Outline/node metadata stays ungated, like the `/items` listing.
* **Alternatives considered:**
  - *Also gate the whole-item read (`GET /items/:id` without `lines`)* — not done here. It changes a shipped contract the UI depends on (item detail, `ImageStrip`), and R35.2 is reworking the cloud gates in parallel. **Open for the owner:** while it stays ungated, the window gate is only partial — an agent can drop `lines` and read the whole file. Closing it is a one-line `check_bytes` call in `items::read_item` once R35.2 settles how item marks gate.
  - *Treat an absolute prefix outside every folder root as malformed* — rejected. It is well-formed and can become valid when a source is added, and the R36.6 contract ("covers no root → matches nothing") stays.
  - *Strip a leading `./` instead of rejecting* — rejected. The row asks for errors that teach the expected form, and silent rewriting hides the mistake from the agent's next call.
  - *Separate `get_node` MCP tool* — rejected per the owner call (nodes fold into the outline).
* **Consequences:** score-neutral — no retrieval code changed, so the golden eval is unchanged. Prose line spans count the cleaned text and can drift from stored-text windows by a few lines. Config, code and markdown spans index the stored text exactly (pinned by the config-span test in `tests/api_read.rs`).

---

## 2026-10-27 — Sensitive content: PII bundled into one Sensitive label, gated by default, opt in to avoid (owner; not implemented — R35.4–R35.6)

* **Decision:** sensitive and PII are one concept, **Sensitive**, chosen per source in the source form. Sensitive content is gated by default on every path where it would leave the machine or reach an agent (cloud embed, cloud answer/rewrite, search/ask/agent compose, `/decisions`/`/actions`, `/media`, and every agent read surface). A person opts in to avoid the gate explicitly in the app, per destination (cloud embedder, cloud answer provider, AI agents). The person at the UI always sees everything.
* **Context:** On 2026-10-13 the owner removed both PII affordances from the UI (the `pii` source label and the per-document PII toggle; `api.ts setItemPii` became a hidden shim). That removal was never recorded here, and `AGENTS.md` kept describing the per-document mark as live, so R35.2 (2026-10-27) enforced a mark nobody can set from the app. R36.15 then showed the read surfaces gate unevenly: line windows and node text are gated, while the whole-file read, section/tag chunk lists and entity windows are not. Agent trust is inferred from `answer_base_url`, which describes the local-mode answer provider, not the agent. The only bypass is two env vars.
* **Alternatives considered:**
  - *Keep three layers (source labels + per-document mark + read gates) and patch R36.19 on top* — rejected: complexity with no UI to drive it.
  - *Remove the cloud gate entirely, keep only `public` for sharing* — rejected: "Sensitive" is a real UI choice people make today.
  - *Gate only when an agent channel opts in via a header* — rejected: open by default, so an agent that improvises a curl reads everything.
* **Consequences:** R35.4 (bundle + schema cleanup; absorbs R35.3), R35.5 (one gate on every surface, `X-Anchor-Client: ui` exempts the SPA, default closed; absorbs R36.19), R35.6 (Settings switches replace env-only trust; env vars stay as admin overrides). **Per-document mark (owner, same day): keep it, derived — not copied.** `ingested_items.is_pii` becomes `sensitive_override` (nullable: `NULL` inherits from the source, `1` marks the document Sensitive; escalate-only, `0` reserved). Effective Sensitive = `sensitive_override = 1 OR source.label = 'sensitive'`, one helper in `common.rs`, read by every gate. Rejected: *copying the source label onto every item* — relabels, new syncs, backup import and future manual marks would drift, and a stored bit cannot tell "inherited" from "set by a person"; *dropping the mark* — loses the per-document hook that future writers (UI toggle, path rules, connector metadata) need, and discards R35.2's plumbing. The gate remains a guardrail against accidental leakage by well-behaved agents and providers, not a boundary against a local process that can read the SQLite file.

---

## 2026-10-27 — Owner answers: 1.6.0 timing, tag threshold, RAM tiers, re-chunk, public scopes

* **1.6.0 ships after R35.5** (not sooner with `get_item`'s whole-file form held back). The new MCP `get_item` puts the ungated whole-file read one tool call away for agents; gating every surface first is the coherent story.
* **`tag_reuse_threshold` kept for now** (R30.10): multi-word cosine reuse stays even though ingest exercises only exact identity. Removing the user-visible setting is a later cleanup, not pre-release churn.
* **Answer-model RAM tiers accepted as shipped** (R45.6): llama3.2:1b any RAM, 3b ≥ 8 GB, qwen2.5:7b ≥ 16 GB, 14b ≥ 32 GB, 32b ≥ 64 GB, unknown RAM → 3b. Agent channel stays un-preselected; the wizard installs only the `search` skill.
* **R36.2 choices accepted:** boot-time synchronous re-chunk, only depth-1/root lists count as main content, identifying values over 32 chars stay out of the breadcrumb.
* **Public scopes drop effective-Sensitive whatever the provider** (R35.5 scope): Sensitive is Sensitive — source label or per-document override, PII or not — derived at the gate entry point from the source mark. Default closed; sensitive content can be opt-in included (R35.6 switches).
* **Folder-picker Chromium collapse fixed** (found in R45.19 browser testing): a grid-item `<button>` with `overflow:hidden` collapses to padding height and clips its own label. Truncation moved to an inner span (`FolderPicker.tsx` `dirLabel`); regression test pins the trio off the button.

---

## 2026-10-27 — R35.4: PII bundled into Sensitive (implementation)

* **Decision:** migration 29 (`bundle_sensitive` hook) converts legacy `pii` labels to `sensitive`, renames `ingested_items.is_pii` to the tri-state `sensitive_override`, and drops the dead chunk PII columns (absorbs R35.3). One rule in `common.rs`: `effective_sensitive` (override OR label, derived at read time) + its SQL twin `effective_sensitive_sql`, with `cloud_gate_reason` gating on the effective value. `Hit.is_pii` and its chunk-column reads are gone. API: `PATCH /items/:id` takes `sensitive_override` (`true` sets, `null` clears, `false` is a 400 — 0 reserved); a present `is_pii` key is a 400 naming the new field; `pii` labels are a 400 naming `sensitive` on create/update. Item JSON carries `sensitive_override` (true|null), entity JSON `item_sensitive` (effective); the audit `blocked` entry for an override is `item_sensitive`. Old backup archives import with `pii`→`sensitive` and `is_pii: true`→override.
* **Context:** the owner decision fixed the shape (bundle, derive-don't-copy, escalate-only); what it left open was the fate of existing marks and the exact contract edges.
* **Alternatives considered:**
  - *NULL every existing row (the row's letter: "migrate to NULL")* — rejected for marked rows: `PATCH is_pii` existed since R35.1 and the setup skill documented it, so API-set marks may exist; NULLing them would silently unenforce real marks. Explicit `1`s survive as the override (escalate-only semantics), unmarked rows become NULL — the asserted common case lands exactly where the row says.
  - *Pure-SQL migration* — rejected: migrations run before `ensure_columns`, so `is_pii`/`label` may be missing on straggler DBs (same reason as R45.10). The hook guards every step on column existence.
  - *One `blocked` entry (`sensitive`) for both causes* — rejected: the audit keeps naming where the mark lives (label vs override), as R35.2 did.
  - *Item JSON as bool (false for inherit)* — rejected: true|null mirrors the column tri-state and the PATCH contract, and stays honest if 0 ever gains meaning.
* **Consequences:** `read_gate.rs` still matches `pii` (dead after migration 29) — R35.5 owns that file and rewrites the gate. Public-scope filtering of overrides stays out (R35.5 scope per the owner's 2026-10-27 answers).

---

## 2026-10-27 — R35.5: gated by default on every surface (implementation)

* **Decision:** one `read_gate::Gate { ui_caller, provider_trusted }` on every path that hands Sensitive bytes to a non-UI caller. The SPA sends `X-Anchor-Client: ui` from `api.ts request()` (exact, case-insensitive match; absent or anything else is an agent) and is never gated; other callers need a trusted provider until R35.6's switches land. Single-item byte reads (`GET /items/:id` whole-file and `lines`, `/nodes/:id?include=text`, `/sections/:id/chunks`, `/entities/:id`, `/entities/:id/context`, `/media/:uid`) refuse with 403 `refusal: "gated_provider"`; multi-item lists (`/tags/:id/chunks`, `/entities`, `/entities/:id/related`, `/review/low-confidence`, search, records) drop the gated rows — each request records one `answer gate` audit event. `public_only` + `/qa/public` drop effective-Sensitive whatever the caller or provider (direct reads 404; ask's nothing-survived text names the public scope instead of "not approved", while the machine contract stays `gated_provider`).
* **Context:** the row named six read surfaces; the entities list, related and the low-confidence queue carry the identical `window_text` bytes as `/entities/:id`, so leaving them open would nullify that gate for a curl-improvising agent — the threat the owner decision explicitly counts. They are gated as multi-item lists (drops, bare-array shapes unchanged). The UI's public-preview checkbox (`AskTab` → `/qa/public`) forces the public scope to be caller-independent: the person previewing must see the share-safe view.
* **Alternatives considered:**
  - *UI header bypasses the ask gate entirely (generation included)* — rejected: the generation call hands bytes to the cloud, which "leave the machine" under the owner decision. ask() gates generation input on provider trust alone and response citations on caller-or-trust, so a UI user with an untrusted provider sees every citation while the model only ever receives allowed context.
  - *403-if-any-sensitive on multi-item lists* — rejected: one marked document would blind an agent to the whole list. Drops mirror search/records; the audit event keeps the withholding visible server-side.
  - *Teaching the skill the UI header* — rejected: the header is the app's own exemption, not an agent channel. The skills document the gated behavior (403/drops); spoofing the header is deliberate evasion, outside the guardrail scope like reading the SQLite file directly.
  - *Query-param exemption for `<img>`* — rejected: inconsistent with the header rule. Thumbnails fetch through `api.assetBlob` (UI header) and render blob URLs, which also feed the open-in-tab link.
* **Consequences:** `read_gate.rs` owns the caller rule (`caller_is_ui`, `Gate::from_request/open/check_bytes`, `check_public` on the effective rule, `item_sensitivity`); R36.15's label-only `gated_label`/`item_label`/`check_bytes(label, trusted)` are gone. Every retrieval fn still takes `source_ids`; `ask`/`search` grew a `ui_caller` bool (transport, not body — MCP/skill POSTs carry no header). Contract in `tests/api_client_gate.rs` (8 black-box tests: agent gated / UI open / trust lifts per path + MCP stdio for `get_item`/`get_entity`) plus caller-split unit tests in `answer.rs`, `items.rs`, `sections.rs`, `entities.rs`, `records_api.rs`.

---

## 2026-10-28 — R47.2: the errors panel dismisses, never deletes; no automatic retention yet

* **Decision:** clearing the System tab's errors & warnings stamps `system_events.dismissed_at` (migration 30); rows stay. Dismissed rows are hidden by default and come back with `include_dismissed` / "Show dismissed" and `restore`. Automatic retention (the row's original step 4: prune dismissed + info rows after 30 days) is not shipped.
* **Context:** `system_events` is three things at once: the panel's source, the audit trail (MCP `ask`/`search` calls — `docs/mcp.md` promises "who asked what is always answerable" — and gate refusals), and an input to item flags (`items.rs` reads `pdf split-word repair` rows). The info level is mostly MCP audit, which is why the panel now starts with info off.
* **Alternatives considered:**
  - *Delete on clear* — rejected: a person tidying the panel would erase audit rows and PDF repair flags.
  - *Prune dismissed + info after 30 days* — deferred: it ends the audit promise after 30 days; that's an owner call (how long audit lives, and whether Team differs from Personal).
  - *Group client-side* — rejected: with a 50-row page, one noisy failure would push everything else out before grouping could fold it.
* **Consequences:** the table grows without bound as before (MCP audit is the bulk); a retention row needs the owner's audit-lifetime answer first. Dismiss/restore take a required JSON body so a malformed request can never mean "dismiss everything".

---

## 2026-10-28 — R47.4: the search skill is `ask-docs`, and people may pick its handle

* **Decision:** the read-only search skill's folder and name become `ask-docs` (owner's pick), and the installer takes an optional `handle` for it. Copies are recognised by a body marker (or the legacy `name: anchorcore-search`), and an install retires every other copy by renaming its `SKILL.md` to `.anchorcore.bak`. The setup skill keeps its fixed name.
* **Context:** after R47.3 triggering comes from the description; the name only matters for `/name` invocation, and `anchorcore-search` was long and product-shaped. A free handle risks two copies that both trigger (breaking the either/or rule) and overwriting someone else's skill.
* **Alternatives considered:**
  - *Rename only, no handle* — rejected: the owner asked for a choice at setup.
  - *Remember the handle in app settings* — rejected: copies placed by hand or by older builds would be missed; scanning the client's skills dir for the marker finds them all.
  - *Delete retired copies* — rejected: installs never delete; `.anchorcore.bak` stops the agent loading it and keeps the file.
  - *Handles for the setup skill too* — rejected: the search skill references it by name, and nobody invokes setup often.
* **Consequences:** the handle is the only request-controlled path piece in the installer, so it is validated to one folder name (no `/`, `.`, or uppercase). Old `search`/`anchorcore-search` ids keep working on the serve and install endpoints.


## 2026-10-28 — R41.7: bind before the database; one transaction per migration

* **Decision:** the server binds its port (tray, browser, `runtime.json`) before `init_db`, and the router serves only `/health` (503 `starting`/`failed`) and the SPA shell until boot finishes. Migrations now run one transaction per schema version instead of one for the whole pending run.
* **Context:** a 5.5 GB upgrade sat silent for ~8 min, and a failed 1.6.0 migration made the app quit with nothing on screen. Per-step progress needs a boundary between migrations; `rusqlite_migration::to_latest` wraps everything in one transaction.
* **Alternatives considered:**
  - *Keep one transaction, report only "migrating"* — rejected: no step N of M, and a failure late in a long run discards every earlier step's work.
  - *Exit with a native dialog on failure* — rejected: three platforms, no tray state, and the person still sees nothing on Linux/Team; serving the error page works everywhere.
  - *Gate per route inside handlers* — rejected: auth middleware already opens the DB; one layer outside CSRF/auth is the only place that keeps every path off the DB mid-migration.
* **Consequences:** every schema version is a valid resting state (each was already a released state), so a failed step leaves the library at version N-1 with earlier steps kept and the next start resumes. A new migration adds a `db::MIGRATIONS` row with its screen wording. Test hooks are debug-build only.
