# R31.1 — Entities + Review confusion audit and redesign proposal

> Status: plan — audit + redesign proposal; see `rust/BACKLOG.md` for whether it landed.

> Live audit run 2026-10-13: release binary on port 8271, fresh temp data dir,
> folder source over the repo `sample/` corpus (10 docs → 10 items, 24 entities),
> Ollama reachable (LLM classify path). No code changed.
>
> **Tree note:** this checkout contains another session's uncommitted UI edits
> (PII toggles removed from `EntitiesTab.tsx`, `types.ts`/`api.ts` deprecations);
> findings may shift when that work lands.

## 1. What the live tab actually shows (sample corpus, first run)

With Ollama up, all 24 entities arrive as `kind=note`, `confidence=1.0`,
`status=verified` (LLM confidences pass the `>= 0.7` auto-verify line in
`pipeline.rs`). Consequences for a first-time user:

- The default filter is `⚠ Needs review`, which matches only
  `unverified`/`disputed` — so the default Entities screen is **empty**:
  `0 / 24 entities · flagged` in flat view (a blank grid, no message at all —
  the flat empty-state checks `entities.length`, not the filtered list), or the
  actively wrong `No entities yet. Ingest a source first.` in grouped view.
  The user's first impression: "it didn't work."
- The Review tab is equally dead: `Nothing needs review.` /
  `No duplicates found.` Two tabs, both empty, on a successfully ingested corpus.
- Without Ollama the picture inverts (rules path): `note` 0.52 / `document` 0.58
  land `unverified` and below the 0.6 low-conf threshold, while `decision` 0.78 /
  `action` 0.72 arrive pre-`verified`. So the queue mechanics flood or starve
  depending on which classifier ran — nothing the UI explains.

## 2. Confusing element inventory (exact labels)

Each row: label as shown → what the user is expected to do → what they will
probably think.

### Entities tab header

| # | Exact label | Expected | Probable interpretation |
|---|---|---|---|
| E1 | `Entities` (tab) | Browse what AnchorCore learned | Jargon. "Rows? People? Companies?" — the word never appears in Ask answers, so nothing teaches it. |
| E2 | `All kinds` + `decision/document/action/note` | Filter by extraction kind | "Kind of *what*?" Live: every row is `note`, so the control looks broken. Worse, obvious decisions (entity #1 is literally a `# Decision:` doc, `**Status:** Confirmed`) arrive labeled `note` — the badges teach distrust. |
| E3 | `⚠ Needs review` (default) | Show unverified + disputed | Reads as "the app found problems." Combines two very different states (never looked at vs actively contested), and on the sample corpus shows nothing (see §1). |
| E4 | `All statuses / Unverified / Verified / Disputed / Stale` | Filter by status | Four engineering states with no legend. `Stale` never matches anything — the pipeline *deletes* replaced entities (`delete_stale_entities`), nothing ever writes `status='stale'`. Dead option. |
| E5 | `0 / 24 entities · flagged` / `N entities · M docs` | Count line | "0 of 24… flagged?" — flagged reads as accused. The `· M docs` suffix appears only in grouped view, unexplained. |
| E6 | `Grouped` / `Flat` toggle | Switch doc-grouped vs row list | Defaults by corpus size (grouped only when > 200 entities) — the screen silently rearranges itself as the corpus grows. Nothing explains the difference; grouped also *sorts* by needs-review count, flat doesn't sort at all. |

### Grouped doc cards

| # | Exact label | Expected | Probable interpretation |
|---|---|---|---|
| E7 | Doc title + `path · item #N` | Identify the document | Title is a filename guess (`source_ref` basename until context loads, then it swaps — the header visibly changes after expand). `item #N` is a database id, meaningless to the user. |
| E8 | Root breadcrumb, e.g. `Runbook: Billing provider migration` | Show doc structure | It's always the *first/root* section of the doc, repeated identically on every entity row — reads as a duplicated subtitle, not navigation (clicking filters nothing; `onSelect` is unset here). |
| E9 | Tag chips, e.g. `billing`, `migration`, `always`, `during` | Show topics | Not the entity's tags — a client-side substring guess (tag name contained in a section path). Junk single-word tags (`always`, `during`, `notes`, `account`) leak through, so the chips look like word salad. |
| E10 | `N entities` | Count per doc | Fine on its own, but combined with E5 the user sees three different counts (header, card, badge) with no stated relation. |
| E11 | `⚠ N need review` | Attention badge | Same "accused" framing as E3; counts disputed (already handled) alongside unverified (never seen). |
| E12 | `avg 100%` | Mean confidence | A mean of model self-scores. 100% everywhere teaches "this number is fake" (it is — LLM default 1.0/0.5, rules fixed per kind). |
| E13 | Kind dots (4px colors, no legend) | Kind mix at a glance | Unlabeled dots; title-tooltip only. Inscrutable. |
| E14 | `✓ Verified` (doc pill) | All rows verified | Appears when *every* row is verified — including auto-verified-at-ingest rows the user never saw. "Verified by whom?" is unanswerable in the UI. |
| E15 | `Show doc ▼` / `Hide doc ▲` / `Open doc ▶` | Expand whole-doc text | Three labels for one toggle, and `Open doc` (verified docs) vs `Show doc` implies different destinations. Same control, different verbs. |
| E16 | `Whole doc — <title>` | Full text header | Fine — but this is the *third* "show me the source" control on one card (see E22), and the only one that says "doc" while others say "context". |
| E17 | `Sections — TOC` + `Art › 12.3`-style paths + `lvl N` | Doc outline | `TOC` and `lvl` are insider abbreviations; the outline lists *all* sections even for a one-section doc, pushing the text down. |
| E18 | `Verify all & collapse` | Mark every row verified, hide rows | "Collapse" is a layout side effect smuggled into a trust action. Runs N sequential PATCHes with one shared spinner state; then hides the rows *for this session only* (local `verifiedDocs` set) — refresh with the default filter and they vanish with no trace ("where did they go?"). |
| E19 | `Dispute doc` | Mark every row disputed | No reason asked, no audit row, no count increment (plain PATCH per row) — yet it has the same QA-exclusion effect as a recorded dispute. Bulk punishment with no trail. |
| E20 | `✓ Verified … doc collapsed, navigate via whole doc above` + `Reopen session` | Post-verify state | "Session" is exposed implementation detail (the hide is local state, not data). `Reopen session` unhides but does not un-verify — half the user expects their click to be undone. |

### Per-entity rows (both views)

| # | Exact label | Expected | Probable interpretation |
|---|---|---|---|
| E21 | `DECISION/DOCUMENT/ACTION/NOTE` badge + `code` badge + `status` badge + `conf N%` | Type + provenance + trust | Four badges in a row, all jargon. `code` (blue, "derived from source code") sits next to kind as if it were a kind. `conf` abbreviates confidence; the value is a classifier self-score, not a fact about the text. |
| E22 | `Show context ▼` / `Hide context ▲` | Show surrounding source text | "Context" vs "doc" (E15/E16) vs Review's `Show context — file, section & neighbours ▼`: three labels for overlapping expansions. Often expands to the *same sentence* as the summary (one-sentence windows), looking like a duplicate (see §3). |
| E23 | `Conflicts ▼` / `Hide conflicts ▲` / `No conflicts flagged.` | Contradiction links | Collapsed-by-default everywhere, so a 24-entity screen carries 24 identical disclosure buttons. The feature (warn on contradiction) is invisible until opened. |
| E24 | `Entity id` input + `Why do they contradict? (optional)` + `Flag conflict` | Link two contradicting entities | **The entity's own id is never displayed anywhere in the UI** — the input asks for a number the user cannot discover. Dead form for anyone who hasn't opened devtools. |
| E25 | `contradicts #N` + `Remove` | Linked conflict | Shows only the other id (again undiscoverable) plus an optional note — no summary of *what* it contradicts. |
| E26 | `Verify` / `Dispute` | Trust actions | `Verify` changes nothing retrieval-side (verified is the absence of exclusion, not a boost) — theater. `Dispute` here is a silent PATCH, while Review's `Dispute` opens a reason box and writes an audit row — same word, different effects (§4). Neither clears nor shows `dispute_count`. |
| E27 | Kind `<select>` (flat view only) | Reclassify | Exists only in flat view — grouped users can't fix the `note`-for-decision mislabels where they see them. Changing kind changes the badge, nothing else. |

### Review tab

| # | Exact label | Expected | Probable interpretation |
|---|---|---|---|
| R1 | `Review` (tab) | Work queue | Indistinguishable in purpose from Entities' `⚠ Needs review` filter — two queues, overlapping membership, different actions. |
| R2 | `Low confidence — needs a human look` | Queue heading | Honest heading, but membership is mechanical, not semantic (see §5): rules `note` 0.52 / `document` 0.58 and LLM-default 0.5 always qualify; LLM 1.0 never does. The queue measures *which classifier ran*, not *what needs a human*. |
| R3 | `[kind] conf N% · <absolute path>` | Row provenance | Absolute filesystem path (`/Users/…/sample/meetings/…`) — noisy, leaks machine layout, and the human-readable title only appears *after* expanding context (it's fetched then). |
| R4 | `⚑ disputed ×N` ("excluded from Q&A" tooltip) | Dispute state | The one label that states its consequence — good. But it can only appear here, and disputed rows *also* sit in Entities' needs-review filter, so the same row nags in two places. |
| R5 | `Show context — file, section & neighbours ▼` + `Show full document (collapsed) ▼` | Expansions | Third context-label variant (see E22). Nested disclosures (context, then full doc inside context) bury the text two clicks deep. |
| R6 | `Looks right` / `Dispute` → `Why is this disputed? (recorded for the audit trail)` → `Record dispute` / `Cancel` | Trust actions | `Looks right` (Review) vs `Verify` (Entities) do the identical PATCH — two verbs, one effect. `Record dispute` writes the audit row + count that Entities' `Dispute` skips — one verb, two effects. The "audit trail" it mentions is **unreadable**: `GET /entities/:id/disputes` has no UI caller; recorded reasons go nowhere the user can see. |
| R7 | `Possible duplicates` + `Entity #A ↔ Entity #B` + `similar summaries` | Merge proposals | IDs without summaries — the user must open two entities elsewhere (by undiscoverable id, see E24) to judge. `similar summaries` describes the detector, not the evidence. |
| R8 | `Merge` / `Not duplicates` | Proposal decision | `Merge` keeps A, repoints chunks, takes the better summary, deletes B — irreversible, unstated. `Not duplicates` is clearer than Entities' vocabulary, at least. |

## 3. Why everything looks duplicated (entity vs doc vs section)

Five distinct repetition sources, all visible on the sample corpus:

1. **Summary == context.** Rule/LLM summaries are often the window's first
   sentence verbatim. Expanding `Show context ▼` then shows the same sentence in
   a box below itself. The user concludes the feature is broken, not that
   summary⊆source by design.
2. **One breadcrumb per row, all identical.** The breadcrumb is the doc's *root*
   section (`sections[0].path`), not the entity's own section — 8 entities from
   the runbook all print `Runbook: Billing provider migration`. Ditto the
   absolute `source_ref` on every row. Doc-level facts repeated at entity grain
   read as copy-paste bugs.
3. **The `document` kind doubles the doc.** `document`-kind entities are
   per-window descriptive extracts, but the label says "document" — next to a
   doc card titled with the same file, they look like the same object twice.
   (Live run had none — all notes — which is its own confusion: E2.)
4. **Entity chunks shadow entities.** Every entity gets its own chunk row, and
   Ask citations / search can surface both the entity-derived chunk and the doc
   chunk with near-identical text. Users cross-checking Ask vs Entities see
   double.
5. **Two tabs, two queues, same rows.** An unverified low-conf entity appears in
   Entities (`⚠ Needs review`) *and* Review (low-confidence queue) with
   different buttons (`Verify` vs `Looks right`). A disputed entity appears in
   both too. Nothing says which place is canonical.

## 4. Which controls belong where (review vs browse vs trust)

| Control (current label) | True job | Belongs to |
|---|---|---|
| `Grouped`/`Flat`, kind filter, `Show doc`, `Whole doc`, `Sections — TOC`, breadcrumbs, tag chips, `Show context` | **Browse** — what do we know, based on what | Entities (reading UI) |
| `⚠ Needs review` filter, `⚠ N need review`, low-conf queue, `Possible duplicates` + `Merge`/`Not duplicates` | **Review** — what needs me, and clearing it | One queue, one place (currently split across two tabs) |
| `Verify`/`Looks right`, `Dispute`/`Record dispute`, `Verify all & collapse`/`Dispute doc`, `Flag conflict`, kind `<select>` | **Trust** — is this true, and what may answers use | Inline on the row being judged, with stated consequences |
| `avg %`, `conf %`, kind dots, `lvl N`, `item #N`, `Entity id` input, `Stale` filter | **None** — engineering exhaust | Hide, or translate into plain language |

Rules the redesign must hold:

- **One queue.** `⚠ Needs review` and the Review tab overlap by construction;
  keep exactly one attention path.
- **One verb per effect.** `Verify` ≡ `Looks right` today; `Dispute` ≠ `Record
  dispute` today. Each distinct backend effect gets one label, used everywhere.
- **Trust actions state consequences.** "Verify" currently promises nothing and
  delivers nothing retrieval-side; "Dispute" excludes from answers without
  saying so (only R4's tooltip does). Every trust button must say what changes.
- **No id-typed navigation.** Anything identified by a bare number (E24, R7)
  must show summaries and link directly.
- **Recorded data must be readable.** Dispute reasons (`/disputes`) and
  relationships (`/related`) have API but no UI — either surface them or stop
  recording them.

## 5. Review-tab mechanics (why the queue misbehaves)

- **Flood/starve by classifier, not by content.** Queue = `confidence < 0.6 AND
  status = 'unverified'`, limit 100. Rules path emits fixed confidences
  (`note` 0.52, `document` 0.58 → always queued; `decision` 0.78, `action` 0.72
  → auto-verified, never queued). LLM path emits whatever the model says with
  0.5 default when absent — and 1.0-happy models (this run) starve the queue to
  zero. Threshold and emitters were never calibrated together.
- **Verified theater, three layers.** (1) `conf >= 0.7` rows arrive
  `status=verified` with no human involved (`pipeline.rs`); (2) pressing
  Verify/Looks-right changes no retrieval behavior (`status_ok` only excludes
  `stale` + optionally `disputed`); (3) doc-level `✓ Verified` inherits both.
  The only trust action with a real effect is Dispute (QA exclusion, default on).
- **Duplicates are IDs-only.** `GET /review/duplicates` returns
  `{id, entity_a_id, entity_b_id, reason: "similar summaries"}` — no summaries,
  no source refs. The UI would need two extra fetches per proposal to show
  anything judgeable. Merge itself is destructive-merge-into-A (B deleted, its
  relationships and proposals cascade-deleted) with no preview or undo.
- **Side note:** `duplicates_handler` inserts proposals as a side effect of a
  GET (re-GETs are stable only because of the `existing` check) — worth
  cleaning if the endpoint survives, but out of R31 scope.

## 6. Review-tab disposition

**Recommendation: fold, don't cut outright — remove the tab, keep the API.**

- The low-confidence queue folds into Entities as the single attention path
  (options B/C below): the same rows, surfaced by sort + badge instead of a
  second screen.
- Per-row `Record dispute` (reason + audit trail) becomes *the* Dispute control
  everywhere, replacing the silent PATCH — one verb, one effect, with the audit
  trail made readable (§4).
- Duplicate proposals become inline cards in Entities (both summaries shown,
  `Merge`/`Not duplicates`), backed by the existing endpoints.
- **API consequences:** keep `GET /review/low-confidence`,
  `GET /review/duplicates`, `POST /review/merge` — they are small, agent/MCP
  useful, and cost nothing without a tab. If R31.2 folds duplicates inline, add
  summaries to the duplicates response (`entity_a_summary`, `entity_b_summary`,
  `source_ref`s) — additive, no break. No endpoint needs deletion; the tab
  removal is frontend-only (`App.tsx` + `ReviewTab.tsx` + its `api.ts` callers
  move, not die).

## 7. Redesign options

### Option A — Keep structure, fix the lies (smallest)

Keep Entities + Review tabs, grouped + flat, all controls. Fix correctness:

- Before: default `⚠ Needs review` → empty screen / `No entities yet. Ingest a
  source first.` After: default `All statuses` (or needs-review only when
  non-empty, with a truthful empty state: `Nothing needs review — showing all
  24.`); both views share one empty-state component.
- Before: `Entity id` input with undiscoverable ids. After: every row shows
  `#id`, conflict input autocompletes by summary, duplicates cards link to rows.
- Before: `Verify`/`Looks right` vs `Dispute`/`Record dispute` verb salad.
  After: one verb set used in both tabs (`Looks right` + `Dispute…` with reason
  everywhere; silent PATCH deleted).
- Before: `Stale` filter, `avg %`, kind dots, `lvl N`, `item #N`. After:
  removed or footnoted.
- Before: duplicates show `Entity #A ↔ Entity #B` + `similar summaries`.
  After: both summaries + source refs rendered (needs the §6 API addition).
- Cost: ~2d, no layout work. Risk: the screen stays dense; the two-queue
  problem (E3 vs R1) survives, just quieter.

### Option B — Language + hierarchy pass (recommended)

Same screens, but rewritten around the three questions, with one queue:

- **Header rewritten:** `Entities` → `What we know`; `All kinds` → `All types`
  with plain words (`Decisions`, `To-dos`, `Facts`, `Notes`); `⚠ Needs review`
  → `Needs a look (N)` showing a *count*; `Grouped`/`Flat` → `By document` /
  `All items` with the default fixed to By-document (no 200-row auto-flip).
- **One attention path:** the Review tab goes away (§6). Its queue becomes the
  `Needs a look` filter + a pinned `Things to check` strip at the top of
  Entities (low-conf rows first, then duplicates-as-cards with summaries, then
  disputed awaiting re-verdict). Row actions there are `Looks right` /
  `Dispute…` (reason required, trail readable via a new `History` disclosure
  per row backed by `GET /:id/disputes`).
- **Provenance demoted to where it's asked:** per-row `source_ref` collapses to
  `filename · section` (no absolute path); root-repeated breadcrumbs move to
  the doc header only; tag chips show only tags actually attached to the shown
  chunk (drop the substring guess + junk words, or drop chips entirely);
  `conf %`/`avg %`/dots/`lvl`/`item #` hidden behind a `Details` disclosure.
- **Trust states consequences:** `Looks right` → `Looks right — stays in
  answers`; `Dispute…` → `Dispute… — removes from answers until re-checked`;
  doc pill `✓ Verified` → `✓ Checked` only when a human clicked (ingest
  auto-verify renamed to `New`, not `Verified` — the single most honest change
  in this option).
- **Bulk actions clarified:** `Verify all & collapse` → `Looks right — all N
  below` (no layout side effect; rows stay visible, filter no longer hides
  them immediately — checked rows dim instead of vanish); `Dispute doc` gains a
  reason box like single disputes.
- Cost: ~1–2w incl. the duplicates-response addition + `History` disclosure.
  Risk: relabeling without regrouping still leaves a long screen — mitigated by
  the `Needs a look` strip answering question (2) above the fold.

### Option C — Regroup around the three questions (fullest)

Restructure Entities into three stacked sections on one screen (or three
sub-tabs), Review tab removed:

1. **`What we know`** (browse): doc cards with titles, TOCs, whole-doc text and
   their extracted items in plain language — no badges, no percentages, no
   trust buttons. Answers (1) what do we know about X and (3) based on what
   (every item one click from its source passage).
2. **`Needs a look (N)`** (review): the single queue — low-confidence items
   with *why they're here* (`Model wasn't sure`, `New since Tuesday`,
   `Possible double of "…"`, `You disputed this`), each with `Looks right` /
   `Dispute…` / `Merge` / `Not doubles` inline and summaries always visible.
   Answers (2) what needs me and what do I do.
3. **`Checked`** (trust log): everything the user verdict-ed, with verdict,
   reason, date, and undo — making Verify/Dispute feel real and reversible.
   Backed by existing statuses + the disputes table, finally readable.
- Conflicts move into `Needs a look` as `Two items disagree` cards with both
  summaries and `Keep both` / `One is wrong…` — the `Entity id` form disappears
  entirely.
- Cost: ~2–3w, the only option touching information architecture. Risk:
  biggest change; but it is the only option where a fresh tester passes the
  B62 one-minute tests without guidance, because the questions are the UI.

### Before / after outline (shared)

```
BEFORE                              AFTER (B shown; C sections it)
Entities [All kinds][⚠ Needs        What we know [All types][Needs a
  review] 0 / 24 entities            look (3)] 24 items
  · flagged [Grouped|Flat]            [By document|All items]
┌ doc card ─────────────────┐       ┌ Needs a look (3) ───────────────┐
│ title · path · item #N    │       │ why-here + Looks right/Dispute… │
│ root breadcrumb (repeat)  │       └─────────────────────────────────┘
│ tag chips (guessed)       │       ┌ Doc: billing-migration-runbook ─┐
│ N entities · ⚠ N · avg %  │       │ 8 items · 2 need a look         │
│ dots · ✓ Verified?        │       │ ┌ item ───────────────────────┐ │
│ [Show doc][Verify all &   │       │ │ FACT · billing-migration… › │ │
│  collapse][Dispute doc]   │       │ │ "Parallel run: mirror…"     │ │
│ ┌ entity ───────────────┐ │       │ │ From: Steps · [Why here]    │ │
│ │ KIND code unverified  │ │       │ │ [Looks right][Dispute…]     │ │
│ │ conf % · abs path     │ │       │ └─────────────────────────────┘ │
│ │ summary               │ └─────────────────────────────────────────┘
│ │ [Show context][Verify]│       Review tab: removed (folded above);
│ │ [Dispute][kind ▾?]    │       /review/* API kept for agents.
│ │ Conflicts ▼ (×N rows) │
│ └───────────────────────┘
Review tab (separate queue,     No absolute paths, no bare ids, no
  same rows, other verbs)         dead Stale option, no fake precision.
```

## 8. Out-of-scope data-model asks (split out, not smuggled)

Found during the audit; each needs its own row, not R31.2 scope:

- **Kind taxonomy accuracy** (decisions arriving as `note`) — already R33.6.
- **Ingest auto-verify** (`conf >= 0.7` → `verified`) + **threshold/emitter
  calibration** (0.6 vs rules 0.52/0.58/0.72/0.78 vs LLM 0.5-default/1.0-happy)
  — one calibration row: verified should mean "human looked", and the queue
  threshold should be set against measured emitter behavior.
- **`stale` status unwritten** — either write it (reclassify marks instead of
  deletes?) or drop the status and its filter.
- **Duplicates GET with insert side effects** — make proposal generation
  explicit if the endpoint survives (§5).

## 9. Decision

**Owner pick: DECIDED (see below, 2026-10-13)** — recommended: **Option B** (language + hierarchy,
Review folded into Entities, `/review/*` kept). Rationale: it fixes the three
worst confusions (§10) and the two-queue split at moderate cost; Option A
leaves the structural problem; Option C is better but 2–3× the cost and can
follow later (B's labels and single-queue survive into C unchanged).

**Decision (owner, 2026-10-13): A + Review fold, twin-tracked with R33.6.** Implement the agreed non-throwaway core — fix-the-lies (default view, empty states, visible IDs, dead options), single attention queue (Review folded into Entities, duplicates inline with summaries, `/review/*` API kept), honest trust states (ingest auto-verify renamed, consequences labeled). Full relabel/regroup deferred to land WITH the R33.6 taxonomy buckets as one coherent pass (relabeling twice is waste). Sequencing: R31.2 queued behind (a) the in-flight code-review-fixes session landing (it owns EntitiesTab/pipeline/etc. uncommitted), (b) R33.6 buckets. R33.6 phase 1 (audit table → owner eyeball) starts immediately in an isolated worktree.

## 10. The three worst confusions (for the handoff)

1. **The default screen is empty on a working corpus** — `⚠ Needs review` +
   auto-verify-at-ingest means first run shows `0 / 24 entities` (flat: blank;
   grouped: the false `No entities yet. Ingest a source first.`). The app looks
   broken when it worked.
2. **Two queues, same rows, different verbs** — Entities' needs-review filter
   vs the Review tab; `Verify` ≡ `Looks right` but `Dispute` ≠ `Record
   dispute`. Users can't learn what anything does because nothing is called the
   same thing twice — and `Verify` changes nothing while claiming a checkmark.
3. **Numbers that identify nothing and prove nothing** — conflict/duplicate
   flows ask for `Entity id`s the UI never displays (dead forms), while `conf
   %`/`avg %`/`✓ Verified` present classifier self-scores and auto-statuses as
   trust signals. The tab simultaneously withholds the numbers users need and
   flaunts numbers that mean nothing.

## 11. R31.2 acceptance notes (A + Review fold, shipped 2026-10-14)

Branch `rust/R31.2`. Frontend-only — no Rust touched, no `/review/*` API
change (the §6 duplicates-response addition was NOT needed: summaries resolve
client-side from the entity list + `GET /entities/:id`).

**What landed.** Default view is now All items (the empty-first-screen lie is
gone); one shared truthful empty state (`Nothing here — N items total` + `Show
all items`). Single attention path: the count line always shows `N need a
look`, the filter offers `Needs a look (N)`, and duplicate proposals fold in as
a `Possible doubles` strip with both summaries, both refs, and a `Merge — keeps
#A` button that states the consequence. The Review tab is deleted (`App.tsx` +
`ReviewTab.tsx`); `lowConfidence`/`duplicates`/`decideMerge` stay in `api.ts`
and `/review/*` stays on the server for agents — the UI calls duplicates/merge
but not low-confidence (the needs-a-look filter over the full list is the one
queue; calling both would re-split it). One verb set everywhere: `Looks right`
(PATCH verified, "keeps this in answers") + `Dispute…` opening a reason box →
`Record dispute` (POST `/dispute`, audit trail readable per row under
`History`). Bulk actions lost their layout side effects (`Looks right — all N
below`, `Dispute all N…` with a reason box, rows stay visible). Every row shows
`#id`; conflict links show the other item's summary with a `#id` autocomplete;
kind re-label sits in both views. Dead/lying controls removed: `Stale` option
(shown only if stale rows actually exist), `avg %`, kind dots, `lvl N`,
`item #N`, the `✓ Verified` doc pill, the `Verify all & collapse` /
`Reopen session` session-collapse. Absolute machine paths collapse to their
last two segments; doc headers show `N items` + `M need a look`. `Show
context` / `Show document` / `Whole doc` labels unified; TOC heading drops
`TOC`, one-section docs skip the outline. Grouped is the default; the toggle
persists in `localStorage` (no more silent auto-flip at 200 rows).

**Relabel (with the R33.6 buckets).** Kinds: Decision / To-do / Note (+ legacy
Document); buckets: Record / Reference / Code; statuses state consequences
instead of claiming a human check: New / In answers / Disputed (with
`⚑ disputed ×N`). All three label helpers (`frontend/src/entityLabels.ts`)
are open-set — any unknown kind/doc_type/status renders titleized, never
blank. `Entity.kind` widened from the 4-way union to `string` to match.

**Deliberate deviation: tag chips dropped, not fixed.** The chips were a
client-side substring guess (tag name contained in a section path) — there is
no chunk→tags endpoint, so true attachment is unknowable without a backend
change (out of scope) or one request per tag (unbounded: 600+ tags measured on
real corpora). The plan §7-B sanctions dropping them; breadcrumbs + sections +
Whole doc keep provenance. If tags return, they need a real endpoint.

**Fresh-tester answers.** (1) *What do we know about X?* — the first screen
lists every item grouped by document (title, ref, breadcrumb, type + bucket
badges), each one click from its source passage (`Show context`) or full text
(`Show document` → `Whole doc`). (2) *What needs my attention and what do I do
about it?* — the header always counts `N need a look`; the filter isolates
them; `Possible doubles` carries Merge/Not-doubles; every row offers `Looks
right` / `Dispute…` with the consequence in the tooltip and the dispute trail
under `History`. (3) *Why is this item here / based on what?* — breadcrumb +
short ref on every row/card, `Show context` with the source passage and
neighbours, `Whole doc` for the full text, `History` for who disputed it and
why.

**Evidence.** `npm run build` (tsc + vite) green, `npm run lint` clean, `npm
test` 64/64 (12 files: 8 new label tests + 10 new tab tests incl. the recorded-
dispute-not-silent-PATCH assertion). Live smoke 2026-10-14 on `:8263` (fresh
temp data dir, `sample/` 10/10, Ollama LLM path → 36 notes, 5 unverified / 31
verified, legacy `documentation`): context keys + full text, sections root
path, PATCH verify, recorded dispute + readable trail, conflict
flag/list/unflag round-trip, staged-proposal merge (B deleted, A kept,
`merged:true`) and dismiss (both kept), `record`/`reference` bucket simulation
alongside legacy rows (`reference:4, record:7, documentation:24` — all render
via the open-set labels), and the served bundle contains the new strings
(`Possible doubles`, `Needs a look`, `Record dispute`, `Looks right`, `Show
all items`, `In answers`) with the old ones gone (`Possible duplicates`,
`Verify all & collapse`, `Reopen session`, `Needs review`). No PII display
re-added. Server stopped after; temp dir outside the repo.
