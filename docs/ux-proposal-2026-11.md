# Design proposal: a calmer, easier AnchorCore (2026-11-02)

This proposal answers the findings in `docs/ux-audit-2026-11.md`. Before proposing changes I read the reasoning behind the current look: `docs/design-system.md`, `rust/docs/decisions.md`, `rust/BACKLOG*.md`, commits `871c8df` and `3d65f4a`, and the handovers `2026-08-28-v1.0.11` and `2026-10-22-release-1.2.0`. Each proposal below either builds on a recorded decision or says plainly which one it would change. Changes to recorded decisions are marked **⚑ owner call**, and each one needs a `rust/docs/decisions.md` entry before it ships.

---

## 0. How the current look came about

| When | Look | Recorded reason | Status |
|---|---|---|---|
| ≤ v1.0.10 | Indigo `#6366f1` on dark, emoji icons, glow | none (defaults) | replaced |
| 2026-08-28, R12.1 | **Stone & Sage**: sage `#4A5A52`, paper `#F2F0EB`, stone | "indigo reads as chatbot/SaaS (Linear/Notion AI), cold for *memory*"; sage "signals archive / foundry / vault — matte, non-AI" (handover v1.0.11) | replaced, but the brand-mark SVG still uses `.mk-sage` / `.mk-stone` |
| 2026-10-21, R44.14 | Site type scale; "mono only for code/terminal/file names"; one body colour, one muted | Site review: too many font roles | site only; the app never adopted it |
| 2026-10-22, R45.15 + `871c8df` | **Archive**: ink `#1F1B16` on cool paper `#F6F5F2`, oxblood `#8C2F2B` = "this is sourced", IBM Plex Sans/Serif/Mono self-hosted, 2 px radius, no emoji or stripes, section marks on site headings, light default + dark set | "Trust over clever… look like a filed document"; citations "*are* the brand"; Plex self-hosted because the privacy notice promises no third parties | **current** |

**Verdict: the direction is right.** Each step moved away from looking like an AI tool, and the Archive principles still hold. The problems come from how those principles were *carried out*:

1. **The dark set drifted into the AI-tool house style.** The light palette is the designed one: ink on paper with a dark oxblood. In dark mode, oxblood becomes salmon `#E39A8F` on warm black `#151412`, which reads as the cream-and-terracotta look of 2025–26 AI products. Because the site and app default to **Auto**, every visitor with a dark OS sees this version first, and that is the version that looks AI-generated. My screenshots were all in dark.
2. **Oxblood is overused, so it no longer carries meaning.** The rule says "citations, `§` marks, links and selected state". In practice it also colours the eyebrow pill, the 1.1/1.2/1.3 step numbers, every link underline, the active nav item and every section number. If a colour appears 30 times on a page, it can't say "this is sourced".
3. **Three type families plus mono labels.** R44.14 limited mono to code on the site. The app still uses mono for micro-labels ("TRY ONE", "ANSWER", "SOURCES · 5"), badges and status.
4. **"Archive" turned into "document" on the marketing page.** `§1–§8` headings (`871c8df`: "Sections numbered with section marks") make the site read as a spec. Section marks belong to the product's citations, where they mean something.
5. **Structure never got a design pass.** The Archive redesign changed the *skin* of all five tabs and rebuilt only Ask. Sources, Records, Settings and System kept the layout they had grown into feature by feature, which is where most of the navigation pain sits.

---

## 1. Principles (keep the four, add three)

Keep everything in design-system §1: trust over clever, show your work, calm density, local-first stated in words. Add:

5. **One meaning per colour.** Oxblood means "sourced", and nothing else uses it.
6. **One thing per screen.** Each page has one job and one primary button. Advanced options go behind "Advanced".
7. **Say it once.** A state such as "no answer model" gets one notice, in one place, in one set of words.

---

## 2. Visual system

### 2.1 Colour

| Token | Today | Proposed | Why |
|---|---|---|---|
| light `--cite` | `#8C2F2B` | keep | Designed and AA-tested (8:1). |
| dark `--cite` | `#E39A8F` salmon | **`#D4877C` → test, or a desaturated brick around `#C98B80`**, ≥ 4.5:1 on `#221F1B` | Removes the terracotta signal while staying recognisably oxblood. Validate with the contrast script before shipping. |
| dark `--paper` | `#151412` warm black | **`#161616` → `#17171A`** (neutral to slightly cool) | Warm black plus cream text is the second half of the AI-tool look. Cool paper is the light-mode intent ("cool paper"), so dark should follow it. |
| links | oxblood + underline | **ink + underline**, oxblood on hover only | Frees oxblood for citations. **⚑ owner call** (changes design-system rule 2). |
| step numbers, eyebrow, active nav | oxblood | ink / `--dim` | Same reason. |
| status colours | green/amber/red/blue/slate | keep, but always paired with a word | Colour alone isn't enough (accessibility). |
| `.mk-sage` / `.mk-stone` | Stone & Sage leftovers | rename `.mk-accent` / `.mk-muted` | Cleanup. |

**Default theme:** website **Light**, with Auto/Dark offered in the footer. **⚑ owner call.** The site is the brand's "filed document" and should show the designed palette first. The app keeps Auto (people choose their own tools' themes), but the switch moves from the header into Settings → Appearance.

### 2.2 Type

| Role | Family | Size | Notes |
|---|---|---|---|
| Page titles, site H1/H2 | Plex Serif 500 | as today | keep |
| Answer text | Plex Serif 500 (only weight bundled) | 16 | Keep (design-system: "serif answers"), but render it properly (§3.3). |
| UI, labels, buttons, section labels | Plex Sans | 14 / 13 | **No uppercase mono micro-labels.** "Sources (5)" in sans 13 semibold. |
| Paths, line ranges, `S1`, code, commands | Plex Mono | 12–13 | Mono only for things you could paste into a terminal (R44.14, now applied to the app). |

### 2.3 Shape and motion
- Keep 2 px radius, no shadows except the one hero offset, and no emoji.
- **Motion:** remove the site's scroll reveal. Show the final frame of the hero demo at load and replay it only on click. Keep app transitions at 0 ms, except the spinner.

### 2.4 Copy rules (add to design-system §5)
- At most one em-dash per screen. Use a full stop or a colon instead.
- No middle-dot chains of three or more items. Write a sentence or a list.
- No slogans in footers.
- Product nouns only: *source, document, passage, citation, decision, action, answer model, agent*. Retire from the UI: *embedding, backfill, compose probe, reclassify, TOC, cosine, component=*. They move to Advanced or to the docs.
- A caveat appears once, at the point where it matters.

---

## 3. App

### 3.1 Information architecture

Today: `Ask · Sources · Records | Settings · System`, plus a project picker, an "API online" pill and a theme switch in the header. None of the tabs has a URL.

Proposed:

```
┌──────────────────────────────────────────────────────────────────────────┐
│ ▣ AnchorCore    Ask   Library   Records   Agents          ● Settings     │
└──────────────────────────────────────────────────────────────────────────┘
  #/ask            #/library     #/records   #/agents          #/settings/...
```

| Tab | Contains | Comes from |
|---|---|---|
| **Ask** | question, scope (project + public-only), answers | Ask + header project picker + the public-only checkbox |
| **Library** | sources list → source detail (documents, images, sync history), Add source, Projects | Sources + jobs list + Unplaced images |
| **Records** | Decisions, Actions (Glossary moves out) | Records |
| **Agents** | Connect Claude Code / Cursor / Codex / opencode (MCP), skills, sharing with agents | System → Agent access + Agent skills + Settings → "AI agents" switch |
| **Settings** | left rail: *Answers · Search · Sensitive sharing · Glossary · Backup · Updates · Status & logs · Appearance · Advanced* | Settings + System + Glossary |

- The header loses the project select, "API online" and the theme switch. **●** is a single status dot: hidden when all is well, amber or red with a tooltip ("Answer model not working") when not, and it links to Settings → Status.
- **Every tab and settings section gets a hash route**, so reload, Back and deep links work. Notices link straight to the fix (`#/settings/answers`).
- **Tabs stay mounted.** Ask history lives in a context, so switching tabs never loses it.
- Agents becomes a top-level tab because the site sells two equal paths: "Your agent writes it" and "A local model writes it". Today one of them is buried in System. **⚑ owner call** (five top-level tabs stay five, but their contents change).
- Glossary moves into Settings because it is vocabulary the person sets up once and edits rarely (R45.13). It isn't a record.

### 3.2 Global notices: one rule

| Situation | Today | Proposed |
|---|---|---|
| No working answer model, local mode | red banner on every tab + 2 yellow boxes + in-answer text | Header dot (amber). On **Ask only**, one inline line above the input: "Answers show matching passages until an answer model is set up. **Set up**". It can be dismissed per session. |
| Agent mode | n/a | Nothing. This is a supported setup, not an error. |
| Source sync failing | banner | dot (red) + a badge on Library |
| Update available | banner | dot + a line in Settings → Updates. No banner (updates are manual-first). |

### 3.3 Ask

```
Ask
┌────────────────────────────────────────────────────────┐ ┌─────┐
│ What was decided about billing, and why?               │ │ Ask │
└────────────────────────────────────────────────────────┘ └─────┘
 Searching: All sources ▾                  Clear conversation

 Who owns billing migration?
 ─────────────────────────────────────────────────────────
 Sarah owns the billing provider migration [S1]. To roll
 back, flip the per-tenant flag to the legacy provider [S2].

 Sources (3)
 ┌ S1  Decision: Billing provider migration approved ─────┐
 │     decisions/d2-billing-provider-approval.md · 1–18   │
 │     "Rollback plan: feature flag toggles back…"   Open │
 └────────────────────────────────────────────────────────┘
 ┌ S2  Runbook › Steps …                                  ┐

 ▸ Earlier: What was decided about instant payments?      (collapsed)
```

- **Keep newest-first.** design-system §4 records it as deliberate ("question reads above its answer"). Show only the newest pair expanded and collapse earlier pairs to one line each. That fixes the "follow-up sits above its answer" confusion without reversing the decision.
- **Scope** ("Searching: All sources ▾") is a menu holding projects plus "Only public sources". This replaces the header select and the "share/MCP-safe" checkbox.
- **Without an answer model, show the evidence view.** Agent mode already renders "Evidence for your agent" as source cards (design-system §4). The no-model local case should render the **same view**, headed "Matching passages", with no answer block and no fake answer text. The server string `"Answer for: …\n\n[No model key configured. Matching context:]"` (`answer.rs`, cut at 1,500 chars, which is why the text stopped at "webhook deliver") should no longer be displayed. The UI switches on `mode`. This removes all four warnings and the raw markdown in one change.
- **Answer rendering:** `answerText.ts` gains headings (rendered as bold run-ins, never `#`) and real ordered lists (`1. 2. 3.`). Model output that hard-wraps lines gets re-flowed.
- **Starter questions:** shown only when the only sources are the bundled samples. Otherwise generate three from the person's own data: the newest decision title ("What was decided about {title}?"), the owner of the newest open action ("What does {owner} own?") and a filename. Clicking a starter **runs** it. Drop "Show the steps for it"; follow-ups are explained once in the placeholder of the second question.
- Every source card gets **Open** (the file, at its lines where possible) and **Copy citation**. Each answer gets a **Copy answer** button.
- **Loading state:** show "Searching 10 documents…" with a progress line, then the answer appears in place.

### 3.4 Library (was Sources)

```
Library                                               [ + Add source ]
┌───────────────────────────────────────────────────────────────────┐
│ Sample corpus            Folder · 10 documents · Internal          │
│ ~/anchorcore/sample      Synced 2 Nov, 10:00 ✓          Sync  ⋯   │
└───────────────────────────────────────────────────────────────────┘
 Projects: none yet. Group sources to scope questions.  Create project
```

- **The list comes first.** "Add source" opens a side drawer with a step per connector type and **visible labels** on every field. The sensitivity choice becomes a labelled radio: *Internal (default) · Public · Sensitive: kept away from cloud models and agents unless you allow it*.
- Each row shows the document count, sensitivity, health (✓ / ⚠ with the reason) and an unambiguous date ("2 Nov, 10:00"). **Delete** and **Reclassify** go in the `⋯` menu. Reclassify is renamed **Rebuild records** with one line explaining when it's needed.
- Clicking a row opens the source detail: documents (drill-down via `GET /items`), images (Unplaced), sync jobs and errors for that source. That gives the job and error rows their missing cross-tab link (R47.2 left them out for exactly this reason).
- Drive's "OAuth access token" field gets an **Advanced** tag and a link to the guide, until there is a real sign-in.

### 3.5 Records

```
Decisions   Actions                     In force ▾   🔍 Search   4 decisions
──────────────────────────────────────────────────────────────────────────
Billing provider migration approved                         Sarah · ⋯
We approved moving to the new billing provider.   d2-billing-provider-approval.md
```

- The title and decision text go on separate lines, with no "Title: Title restated" duplication.
- The status tag only shows when it differs from the filter (for example "Superseded" in the All view).
- **Dispute…** and **Restore** move into the `⋯` row menu.

### 3.6 Agents (new tab)

```
Agents
Let a coding agent search your library and cite it.

  Claude Code     detected       [ Connect ]      ✓ connected
  opencode        detected       [ Connect ]
  Cursor · Claude Desktop · Codex   not found on this computer   (show)

  Can agents see Sensitive sources?   Off   [ change ]

  ▸ Manual setup (command / JSON)        ▸ Skills (when MCP is blocked)
```

- Show detected agents only. Collapse the undetected ones into one line.
- The command, JSON and full paths stay behind "Manual setup".
- List the real tool set (all 12 MCP tools, in plain words: "search, ask, read documents, list decisions and actions…").

### 3.7 Settings

A left rail, one section per page, **autosave with ✓ Saved** (no single Save at the bottom). Order:

1. **Answers**: "Who writes the answers?" (Agent / Local model). In Local, pick the model; saving runs the test automatically and reports *Working* or *Not working: reason*. Reasoning effort goes under Advanced. The terms "compose probe" and `mode: composed` are retired from the UI.
2. **Search**: one control: *Built-in model (recommended) / Ollama / Cloud endpoint*. The second provider select only appears when it applies. The offline-files path goes behind "Install without internet".
3. **Sensitive sharing**: today's card, unchanged in behaviour.
4. **Glossary**.
5. **Backup & restore**. Drop "review state" and "users" from the copy.
6. **Updates**: the check button and the startup opt-in together, in one place.
7. **Status & logs**: today's System cards, showing **only rows with a problem**, plus jobs, errors and the log download. Empty cards ("Scheduler tasks", "Retrieval latency: no retrieval yet") are hidden. "redacted" is never printed.
8. **Appearance**: Auto / Light / Dark.
9. **Advanced**: tag reuse threshold, but first check whether it still does anything after R45.18; if it doesn't, delete it.

"Show welcome wizard" moves to the Ask empty state ("New here? **Take the tour**") and to Advanced.

---

## 4. Website

### 4.1 Structure (8 numbered sections → 6 plain ones)

```
Header:  ▣ AnchorCore      Product  Agents  Pricing  Docs  GitHub     [Download for macOS]

1  Hero      H1 + one sentence + [Download for macOS · {size}]  "Using a coding agent? Set it up →"
             Right: a real screenshot of the app answering (static), with a small "Watch it" replay.
             Under: "Folders · Google Drive · Jira · Linear · Miro · any JSON API" as plain text.
2  How it works   three steps, no numbers, with one product screenshot per step (Sources, Ask, citation)
3  For coding agents   the copy command + one MCP sentence + link to Agents docs
4  Private by design   3 bullets (index on disk · no telemetry · Sensitive sources stay local) + "Every network call →" /security
5  Pricing   one line per tier: Personal free · Team self-hosted (quote) · Hosted (waitlist)
6  FAQ + Download   FAQ, then the download block (OS-specific button, other OS links, checksum note)
Footer: one shared footer on all pages, without a version or slogan. Theme switch lives here.
```

- **Drop:** `§` counters (**⚑ owner call**, this reverses part of `871c8df`), the eyebrow pills, the "Sections tree" figure (it explains internals; the screenshot shows the result), scroll reveal and the header theme switch.
- **Download button:** name the OS and size; the label and the target are the same file. The hero button downloads directly (it currently only scrolls). Show the phone email form only to phones, which `detectOs` already identifies.
- **Founder note:** replace the placeholder with a named, specific note: who you are, one real incident ("we lost the reason we chose X and re-did it six months later"), and a photo. Otherwise remove the section. A generic note does more harm than none.
- **Trust section:** keep the full outbound-call list on `/security` (it's excellent there), and put three bullets plus the link on the home page.
- **Corrections:** remove `pii` everywhere (R35.4), replace "v1 local" with nothing, fix the `/team` form option ("Docker on **your** infrastructure"), and rename the footer link "Trust & verification" to "Privacy & security".
- **Guides:** build `docs/mcp.md` and `docs/backup.md` into `/guides/*` with the existing generator so readers don't jump to GitHub mid-evaluation. Keep GitHub links for code only.

### 4.2 Hero copy (keeps the R44.9 positioning)

> **Ask your files. Get the answer and the exact source.**
> AnchorCore searches your folders, Drive, Jira and Linear on your own computer and answers with the passage it came from.
> [Download for macOS] · Free for personal use · Windows and Linux too
> Using Claude Code, Cursor or Codex? **Connect your agent →**

There are no em-dashes, no eyebrow and no middle dots in the button area.

---

## 5. Rollout

| Wave | Contents | Size | Needs owner call |
|---|---|---|---|
| **1. Stop the bleeding** | hash routes + keep tabs mounted; evidence view for no-model; one notice; starters run and are data-aware; remove stale copy (pii, v1, MCP tools, review state, "redacted") | ~2 d | none |
| **2. Visual tune** | dark palette retune; oxblood → citations only; mono labels → sans; site default Light; theme switch moves; em-dash pass; rename `.mk-*` | ~1.5 d | links in ink · site Light default |
| **3. App IA** | Library (list-first, drawer, source detail), Agents tab, Settings rail + autosave + Status folded in, Records row menu | ~4 d | the IA change |
| **4. Website structure** | 6 sections, real screenshots, OS button, founder note, shared footer, guides for MCP/backup | ~2 d + owner copy/photo | drop `§` |
| **5. Docs** | rewrite `docs/design-system.md` §2–§5 for the new rules, plus one `decisions.md` entry per ⚑ | 0.5 d | n/a |

Wave 1 has no design risk and fixes the worst of the audit, so it should go first. Waves 2–4 can run in parallel worktrees: the site and the app share only tokens, and token changes land in `theme.css` and `website/src/style.css` in the same commit (design-system rule 5).

---

## 6. Owner calls needed (summary)

1. **Links in ink; oxblood only for citations.** This changes design-system rule 2 (which currently allows links, `§` marks and selected state).
2. **Website defaults to Light** (Auto stays available). The app stays on Auto.
3. **Drop `§` section numbering on the marketing site** (it stays in citations). This partly reverses `871c8df`.
4. **App IA:** Library / Agents tabs, with System folded into Settings.
5. **Retune dark mode** to neutral/cool paper and a desaturated oxblood.

None of these touches the "never reintroduce" owner decisions in `AGENTS.md`: model-free ingest, people-owned vocabulary and captions, one Sensitive concept, two answer modes and manual-first updates all stay exactly as they are. The proposal only changes how those decisions are presented.
