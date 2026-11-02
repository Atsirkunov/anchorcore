# UX and brand audit — app and website (2026-11-02)

Scope: the live site at anchorcore.dev (home, /team, /hosted, /guides, /vs/notebooklm, /security), its source in `website/`, and the app v1.6.6 at `127.0.0.1:8000` with the bundled sample corpus synced, in local mode with no working answer model. I read the frontend source in `frontend/src` (not the uncommitted `fluid*` experiment). Desktop only, at 1440 px: I could not resize the browser to phone width, so mobile is judged from the CSS alone.

Severity: **P0** hurts first impressions or loses work · **P1** real friction · **P2** polish.

---

## Top 10 to fix first

| # | Sev | Where | Problem | Fix |
|---|---|---|---|---|
| 1 | P0 | App · Ask | With no answer model, one answer shows **four** warnings (red banner, two yellow boxes, `[No model key configured. Matching context:]` in the body), then raw markdown (`# Decision`, `## Steps`, broken `1. 1. 1.` lists, text cut mid-word: "webhook deliver"). | One calm notice and a "Set up" link. Render excerpts as cited source cards, not a fake answer. |
| 2 | P0 | App · nav | Switching tabs **erases the Ask conversation** (`AskTab` keeps `turns` in local state and unmounts). Confirmed: ask, go to Sources, come back, and it's gone. | Lift turns into context or keep tabs mounted. |
| 3 | P0 | App · nav | Tabs are React state only: no URL. Reload always lands on Ask, Back leaves the app, and you can't link to Settings. Error messages say "Settings → Answer model", but nothing can take you there. | Hash routes (`#/settings/answers`), with deep links from warnings. |
| 4 | P0 | App · Ask | The "Try one" chips are hard-coded sample-corpus questions ("Who owns billing migration?") shown for **any** data. A clicked chip only fills the box and doesn't run. "Show the steps for it" means nothing as a first question. | Show chips only when the sample is the only source, or build them from the person's records. A click runs the question. |
| 5 | P0 | Web + app | The `pii` label is still advertised (home matrix row, /team, /vs, llms.txt; 10 mentions in HTML), but R35.4 merged it into `sensitive`. A security-minded reader who checks finds the site and the app disagree. | Remove `pii` everywhere and describe the one Sensitive concept plus the per-document override. |
| 6 | P1 | App · System | System is a junk drawer: health cards, MCP connect, agent skills, backup, updates, errors, jobs and logs on one scrolling page. Agent setup, a core path for half the audience, sits behind "System". | Split out an **Agents** tab (MCP + skills) and a **Data** page (backup, jobs, logs). Keep System for health only. |
| 7 | P1 | App · Settings | The core choice ("Who writes the answers?") comes 4th, below embeddings, the cosine tag threshold and Updates. A single Save sits at the very bottom of a long page. | Order the page answers → search → sharing → advanced. Save per card or autosave. |
| 8 | P1 | Website | The page is numbered like a spec: §1–§8 headings, plus 1.1 / 1.2 / 1.3 on the steps. It reads as a gimmick and makes the landing page feel like a document. | Drop the counters on the marketing page. Keep § marks in the product demo, where they mean something. |
| 9 | P1 | Website | Footer on the subpages says "**v1 local**" (the app is 1.6.6) and ends with the slogan "Private by default · cited by design". The home footer is different. | One shared footer, with no version (or one injected at build time) and no slogan. |
| 10 | P1 | Web | The download button never names the OS it downloads. "macOS, Windows and Linux… Also: Windows · Linux" leaves you to infer you get the Mac build. "On a phone? Get the link by email" shows on desktop. | "Download for macOS (DMG, 120 MB)". Show the phone form only on phones. |

---

## 1. Hard to navigate

### App

- **No URL state** (P0, `App.tsx`: `useState<Tab>("ask")`). No deep links, reload or Back behaviour, or bookmarks. See Top 10 #3.
- **State loss on tab switch** (P0). Conditional render (`{tab === "ask" && <AskTab/>}`) throws away answers, unsaved Settings edits and half-filled source forms.
- **Primary/secondary split is weak.** Settings and System are dimmed beside Ask/Sources/Records and set off by a thin divider, but they hold the most important setup. A first-run person who skipped the wizard has no visible path to "connect my agent".
- **Topics duplicated across tabs.** Updates live in both Settings (startup opt-in) and System (check button). Embeddings sit in Settings (config) and System (two cards: "Ollama up · embeddings" and "Embeddings · provider"). The banner has two buttons, "Set up models" and "Settings", that go to roughly the same place.
- **Sources page order is upside down.** First the Projects creator, then an always-open Add-source form, then the list of what you have. The list should come first, with "Add source" as a button that opens the form.
- **Records hides Glossary.** Glossary (people-owned vocabulary, R45.13) is a third sub-tab under Records. Nobody looks for a glossary under "Records".
- **Ask history is newest-first,** so a follow-up question appears *above* the answer it follows. Each turn reads fine on its own, but the conversation reads bottom-up.
- **Project scope is a bare select ("All sources")** beside "API online". It scopes Ask only, but sits in the global header, so it looks as if it filters Sources and Records too.

### Website

- **Nav labels vs. content.** The nav has 5 links, but the home page has 8 sections. "What leaves your machine", "Why I built it", FAQ and Download are only reachable by scrolling. The footer calls the trust section "Trust & verification", but its heading is "What leaves your machine".
- **The hero's secondary CTA "For coding agents"** jumps to a section titled "Who writes the answer". Different words lead to a section that covers both audiences.
- **Two identical "Download, free for personal use" buttons do different things.** The hero one scrolls to #download and the bottom one starts a download (`main.ts` only rewrites `cta-roadmap`). People click the top one expecting a file.
- **Guides are fragmented.** There are 3 cards on home, an index page with 4, and the rest of the docs (backup, MCP) link out to GitHub markdown. A reader moves between styled pages and raw GitHub views with no way back.
- **Many links leave the site** (GitHub blob links for benchmarks, docs, LICENSE). Each one drops the reader into a code host mid-evaluation.

---

## 2. Looks AI-generated ("slop" signals)

These are patterns readers now link to generated sites and generated UIs. Any one of them is minor. Together they make the product look templated.

- **Em-dash density.** 70 em-dashes in the site HTML and 175 in app UI strings. Examples: "Public-only — answer from public sources only (share/MCP-safe)", "Automatic — Ollama/endpoint below when it serves, else the built-in model", "No answer model ran — this is matching context…". Prefer full stops, colons or parentheses. Aim for under one dash per screen.
- **Monospace eyebrow pill above a serif H1** ("Private AI search · runs on your computer", "FOR TEAMS · SELF-HOSTED", "HOSTED · COMING SOON"). This is the default hero of countless generated landing pages.
- **The warm-dark palette** (`#151412` paper, cream ink, salmon `--cite`), IBM Plex Serif headings, a terminal-session demo panel and a "Replay" button. Each choice is defensible, but together they look like the house style of AI-tool sites in 2025–26. The `--cite` colour carries a meaning ("this is sourced") that a visitor can't decode.
- **Middle-dot strings everywhere:** "AnchorCore v1 local · free for personal use · Team self-hosted on request · Hosted soon", "Tools: ask + search (read-only) · audited as component=mcp", "approved · billing · migration · provider".
- **Slogan footers** ("Private by default · cited by design"), and parallel triads ("Connect… / Indexed… / Answers point…"; "Personal / Team / Hosted").
- **Hedged, over-qualified copy** that sounds like a model covering itself: "larger libraries are projected, not yet measured"; "We did not find this in Google's documentation; check their docs."; "Check Google's documentation for their citation format." Honesty is good, but put the caveats in one place and don't repeat the disclaimer in every row of the NotebookLM table.
- **Leftover scaffolding visible to readers.** The `index.html` comment on "Why I built it" says "to be replaced with the owner's own story". The section is 3 generic sentences ("Every team I worked on lost the why…") with no name, photo or specifics. Founder notes like this are the most commonly generated paragraph on indie sites.
- **App: uppercase micro-labels in mono** ("TRY ONE", "ANSWER", "SOURCES · 5"), plus `[S1]` tokens that are brackets rather than designed chips.

**What would read as human-made:** a real screenshot of the app rather than a mock, a named founder with a photo and one concrete story, product-specific vocabulary instead of em-dash glosses, and fewer, shorter sections.

---

## 3. Harms the image

- **"redacted" printed as data** (System → API card, under the version). It looks like a bug or a censored field. Hide the row or say "Data folder: hidden from the UI".
- **The contradictory answer-model status** the first-run user sees: System says "Answer model: missing · gpt-4o-mini", while Settings shows "Local Ollama (free, private)", and the answer body says "No model key configured". That is three different explanations of one state, and the gpt-4o-mini default suggests a cloud dependency the site says you don't need.
- **Stale or wrong claims in the UI:**
  - MCP card: "Tools: ask + search (read-only)". The server exposes 12 tools (`list_items`, `get_item`, `get_outline`, `decisions`, `actions`, …).
  - Settings → TOC & Tags: "per-section keywords are proposed and reused if cosine > threshold". This is model-ish language, while the pinned decision is that ingest is model-free (R45.18). Check whether the knob still does anything, and remove it if not.
  - Backup card: "carries your memory (sources, documents, entities, review state)". The review queue was removed (R45.13).
  - Site: the `pii` label (Top 10 #5), "v1 local" (Top 10 #9).
- **Debug paths on screen.** System → Manual setup shows the full `target/debug/anchorcore` path and data dir, and Settings prints the full model-cache path inline in body text. In a release build this is the `.app` path, which is better but still a wall of text. Collapse it behind "Show command".
- **Developer jargon on core screens:** "API online", "mode: agent_compose", "Run compose probe… Only a probe reporting mode: composed means answers really work", "audited as component=mcp", "Reclassify", "Embedding backfill", "Retrieval latency", "Scheduler tasks" (an empty card with no value), "Include jobs, events and users" (users, in a single-person app?). Each one tells a non-engineer the app isn't for them.
- **Red error banner on every tab** in a supported configuration. Agent mode and excerpts-only are legitimate setups, but the app presents them as broken. Use neutral styling and a dismiss button, and show it on Ask only.
- **Unsigned Windows build, with the SmartScreen workaround on the home page.** It's honest, but it sits right under the button. Move it to the Windows link's tooltip or the guide, or sign the build.
- **The email capture sits next to "This site sets no cookies and runs no analytics."** That's fine legally, but the waitlist and phone forms on a privacy-first page get a second look. Say where the email goes (provider) in one line.
- **Legacy redirect code shipped to every visitor** (`?track=`, `#guide-*`). This is not visible, just bundle noise.

---

## 4. Bad UX

### Ask (the main screen)
- Four stacked warnings per answer, raw markdown, broken numbering and truncation (Top 10 #1). The excerpts body also repeats the Sources list below it, so the same text appears twice.
- "Answer for: Who owns billing migration?" restates the question that is already the heading.
- The answer body is set in serif with mixed sizes, while the rest of the app uses sans, so it looks like pasted content.
- The "Public-only" checkbox sits under the input on every question. It is an expert option worded in jargon ("share/MCP-safe"). Move it into a scope menu with the project picker.
- "Clear context" plus "1 question in context — follow-ups like… work" is good, but it only appears after the first ask, which pushes the input down.
- No loading state is visible in the first seconds (no skeleton or "searching 10 files…"), and there's no way to copy an answer or open a cited file.

### Sources
- Inputs have placeholders only and no labels (2 of the 3 inputs on Ask are unlabeled too). The sensitivity select ("Internal") has no label explaining what it does, even though it is the app's main privacy control.
- "Folder path (absolute)": the Browse button is good, but "absolute" is jargon.
- Each row shows "folder · last sync: 02/11/2026, 10:00:52", which is ambiguous between D/M and M/D, and doesn't show document count, health or label. "Delete" sits right next to "Sync now" and "Reclassify". Move destructive actions into a menu.
- "Reclassify" is unexplained.
- Drive asks for a raw "OAuth access token (Bearer)". People can't get one without developer tools, and it expires. Either label the connector as advanced or don't show it.

### Records
- Titles repeat themselves: "Billing provider migration approved: We approved moving to the new billing provider." Show the title, with the decision text as a secondary line.
- An underlined "Dispute…" link sits on every card, the most prominent action on a read-only list. Put it in a row menu.
- Every card is "ACCEPTED", in a green tag that adds no information when the filter is already "In force".

### Settings
- The section order is wrong, and there's one Save at the bottom (Top 10 #7). Leaving the tab loses edits (Top 10 #2).
- Embeddings are configured with **two** selects stacked (Engine: Automatic/Built-in/Ollama-or-cloud, then a provider select). The second one is still visible when "Built-in" is chosen.
- A cosine threshold is exposed as a top-level setting. Move it under "Advanced", or remove it.
- The answer model flow has "Save → Test → Probe" as three separate steps the person has to run. Do it in one step: Save tests automatically and shows ✓/✗.

### System
- 9 status cards, some with no value ("Scheduler tasks", "Retrieval latency: no retrieval yet"). Show only what is wrong, and put a single "All good" when nothing is.
- MCP rows show "Connect" for agents that are *not detected* (greyed, but still buttons). Hide them or explain what will happen.
- "Show welcome wizard" sits here and nowhere else.

### Website
- The hero demo panel is a tall empty box until the animation types out (about 3–4 s at load), so the first screenshot of the page has a large blank rectangle beside the headline. Render the final frame first and replay on click.
- The trust section is long, dense prose in a bordered box, with a 4-column ✓/✗ table next to it. It's good content, but it's the longest section on the page and comes before pricing. Lead with a 3-bullet summary and link to /security.
- Pricing "By quote" / "Soon" in large type next to "Free" makes two of three tiers look unavailable. Lead with Personal and put the others in a single line.
- Scroll-reveal fades content in. In the screenshots, headings and body sat at low contrast mid-transition. It's an enhancement, but it makes the page feel slow on fast scrolling. Shorten it to ≤150 ms or remove it.
- The /team form option "Self-hosted (Docker on our infrastructure)": "our" reads as AnchorCore's infrastructure. Say "your".

### Accessibility quick checks
- Global `:focus-visible` outline exists (good).
- Unlabeled inputs across Ask and Sources (see above).
- Status is conveyed by colour only in several places ("up" in green, "not working" in amber, ACCEPTED tags).
- The tab bar is plain buttons with no `role="tablist"` or `aria-current`. The site's demo tabs do this correctly, but the app doesn't.

---

## Suggested order of work

1. **Stop losing state and add URLs** (Top 10 #2, #3): small change, large effect.
2. **Fix the no-model answer view and banner** (#1): this is the first thing every agent-mode or fresh user sees.
3. **Correct the stale claims** (pii, v1, MCP tool list, review state, the tags/cosine text): copy edits only.
4. **Restructure the IA:** add an Agents tab, slim System down, reorder Settings, put the Sources list first.
5. **De-template the website:** drop § counters and eyebrow pills, cut em-dashes, show a real app screenshot, write a real founder note, use one footer and an OS-named download button.
6. **Polish:** labels, dates, Records cards, colour-only status.
