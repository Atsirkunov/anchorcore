# AnchorCore — Design System & Messaging

> For a non-UX owner: copy-paste tokens + rules. No taste required — pick the preset, don't invent.
> App and website share one look (**Archive**, 2026-10-22): ink on paper, IBM Plex, oxblood for citations. The app is light by default with a matching dark set, chosen by the header Auto / Light / Dark switch (Auto follows the OS).

---

## 1. Brand principles

* **Trust over clever.** You store company memory. Look like a filed document, not a game. No gradients, no glow, no emoji.
* **Show your work.** Every answer cites source + section. Citations are first-class UI, not footnotes — and the only thing that gets the accent colour.
* **Calm density.** Dense but scannable: `14px` body, `12-13px` meta, `16px` serif answers, generous section gaps.
* **Local-first signal.** "Private by default" is stated in words where it matters, not decorated.

---

## 2. Color — one palette, light + dark

Archive: ink `#1F1B16` on paper `#F6F5F2`; oxblood `#8C2F2B` means "this is
sourced" (citation marks, `§` numbers, links, selected state). Primary buttons
are ink, never oxblood (red reads as "delete" on a button).

### 2.1 Tokens (single sources of truth)

App tokens are CSS variables in `frontend/src/theme.css` (`--ac-*`, light in
`:root`, dark under `:root[data-theme="dark"]`, set by `themeMode.ts` + an inline
pre-paint script in `index.html`; the choice is stored as `theme-mode` in localStorage); components read them through
`frontend/src/theme.ts` (`theme.*` = `var(--ac-*)` strings). Site tokens live in
`website/src/style.css` (`:root`).

| Role | App light (`--ac-*`) | App dark | Site (`style.css`) |
|---|---|---|---|
| page bg | `bg #F6F5F2` | `#151412` | `--paper #F6F5F2` |
| card / panel | `bg-card #FFFFFF`, `bg-elevated #FFFFFF` | `#221F1B`, `#1C1A17` | `--surface #ffffff` |
| borders | `border #D6D1C7`, `border-soft #E6E2DB` | `#3D372F`, `#2E2A24` | `--border #DCD8D0` |
| text | `text #1F1B16`, `text-muted #4F473D`, `text-dim #6B6257` | `#ECE8E1`, `#C4BBAE`, `#A1978A` | `--ink`, `--dim #5E554A` |
| primary button | `accent #1F1B16` on `#FFFFFF` | `#ECE8E1` on `#151412` | `--ink` |
| citations / links | `cite #8C2F2B`, `cite-soft #F3E6E3` | `#E39A8F`, `#3A2421` | `--cite #8C2F2B`, `--cite-soft` |
| status | `green #2F5D46`, `amber #85550F`, `red #A3332C`, `blue #2F5577`, `slate #4E5B69` | lighter tints | (n/a) |
| type | `--ac-sans` / `--ac-serif` / `--ac-mono` = IBM Plex Sans / Serif / Mono | same | `--sans` / `--serif` / `--mono` |

**Rules for non-designers:**
1. Never invent a hex. App: only `theme.*` (add a token to `theme.css` + `theme.ts` first). Site: only `:root` vars.
2. Oxblood only for citations, `§` marks, links and "selected". Errors use `red`, which is brighter.
3. No coloured left-border stripes on cards; say the status in words (a coloured mono label).
4. Danger bg/border only for banners and destructive buttons.
5. When changing a shared hue, change `theme.css` and `website/src/style.css` in the same commit.

### 2.2 Contrast (WCAG AA)
* `text #1F1B16` on `bg #F6F5F2` ≈ 15:1 ✓ · `text-dim #6B6257` on `bg` ≈ 5.6:1 ✓
* `cite #8C2F2B` on white ≈ 8:1 ✓ · dark `cite #E39A8F` on `#221F1B` ≈ 7:1 ✓

---

## 3. Typography & spacing

* **Fonts:** IBM Plex, bundled with the app and the site (`@fontsource/*`, latin subsets) — the app works offline and the site loads nothing from third parties. Sans for UI, Serif for headings, questions and answer text, Mono for paths, line ranges, badges and labels.
* **Scale:** `11` mono labels / `12` meta / `14` body / `16` serif answer / `20` question / page titles serif.
* **Radius:** `radius = 2` from `theme.ts` everywhere; `50%` only for dots and spinners.
* **Spacing:** `8` tight, `12` card gap, `16-28` between answers/sections.

---

## 4. App UX

Five tabs (Ask, Sources, Entities, Settings, System). Header: brand mark, text tabs with an ink underline on the active one, one ink rule under the bar.

**Ask (R-archive, 2026-10-22):** each question reads above its answer (newest pair first). Local mode: serif answer with `[S1]` marks (mono, oxblood, hover highlights the source card), then numbered source cards: breadcrumb title, path relative to the folder source + `§` + line range (mono), one excerpt. Agent mode: no answer block; the same cards are the content ("Evidence for your agent"). `answerText.ts` strips `[match N]` marks and renders a safe markdown subset (paragraphs, lists, bold) as React nodes — never HTML.

**Standing micro-rules:**
* Primary button = ink; secondary = light fill + border.
* Empty states keep faint text + one-line hint.
* Citations: white card, `S1` in oxblood mono — this *is* the brand.

---

## 5. Messaging — website + app copy (one voice)

**Positioning (R44.9, 2026-10-21):** *Ask your files. Get the answer and the exact source.* One category noun — **private AI search** — and one result noun — **cited answer**. "Memory", "second brain" and "knowledge base" are retired from the website (the glossary lives in `website/README.md`); the in-app Ask tab heading still says "Ask your memory" and is a separate follow-up.

### 5.1 Website hero

**Master headline:**
> **Ask your files. Get the answer and the exact source.**
> A private AI search app. Connect your folders, Drive, Jira, Linear — or any JSON API — and ask in plain language. It finds the decision, who made it and why, and shows the page it came from. Your index stays on your machine. Answers are generated locally unless you connect a cloud model.

**One label per action, everywhere:** *Download — free for personal use* · *Watch it work* · *Join the waitlist* (Hosted) · *Request a quote* (Team).

**Audiences are pages, not a toggle (R44.16):** the hero is Personal only. `/team` is a quote-request page (no price, range or fee structure; the pricing model is undecided — `rust/docs/decisions.md`), `/hosted` is a one-field waitlist. `?track=team|hosted` redirects to them. The hero's right-hand card is an animated Claude Code session (terminal card) asking AnchorCore a question over the sample dataset (R44.11).

**Claims discipline:** the site only claims what the repo supports — no SSO/roles (R43.18), no address guard on the REST connector, no analytics/telemetry, keychain on macOS/Windows only (encrypted-file fallback elsewhere), Windows/Linux builds not code-signed. Evidence lives on `/security` and the landing `#trust` section.

**Alternates** if the master headline fatigues:
* B: *AI is smart. It just doesn't remember working at your company.*
* C: *Context infrastructure for agents.*

### 5.1b App copy — mirror the split (planned, not implemented)

Track-aware app copy (Ask empty-state hint, onboarding examples switching on
Personal/Team) is designed but **not built** — nothing in the app reads the
site's track today. Note the constraint: site `localStorage` can't reach the
app (different origins), so the handoff must be a `?track=` URL param on the
app URL, not shared storage.

### 5.2 Website structure (light theme, same accent)

1. **Hero** — headline + sources strip + download buttons + Claude Code terminal card (sample dataset) + a two-path strip (use the app ∥ plug into your coding agent).
2. **Problem → Solution** — 3 columns: *Fragmented knowledge* → *Lost reasoning* → *AnchorCore: every answer shows its source*.
3. **How it works** — 3 steps with icons + plain pipeline line (`connect → read → file → ask → cited answer`), sections visual, relationship visual, sealed-by-design privacy strip.
4. **Live demo** — connect `sample/`, 4 questions answered (scripts in `docs/sample-dataset.md`); browser + MCP columns; abbreviated agent-session replay.
5. **Guides** — entry cards linking to prerendered guide pages `/guides/*` (R44.15, built from repo markdown at build time), plus a projects card. A `#trust` section sits before the demo; `/security`, `/vs/notebooklm`, `/team` and `/hosted` are separate pages.
6. **FAQ, Pricing, Roadmap** — FAQ (JSON-LD generated from the visible text), 3 tiers without prices (Team = request a quote), download + waitlist capture.
7. **Privacy notice + Footer** — GDPR notice section; `v1 local · free for personal use · Team self-hosted licensed · Hosted soon.`

**Voice:** short, factual, no superlatives. Same as app: `No relevant knowledge found yet.` not "Oops!"

### 5.3 App microcopy (already good — keep)

* `Ask your memory` — perfect, keep.
* Follow-up hint ("N questions in context — follow-ups work") — excellent, keep.
* Offline banner (`modelHealth.ts`): `Ollama is offline — …` followed by what actually degrades for the current mode (search keyword-only, local answers basic) — keep it mode-aware.

---

## 6. Implementation record

1. ✅ **`frontend/src/theme.ts`** created — all app color through tokens, zero literal hex elsewhere.
2. ✅ **Website** uses the same palette via `website/src/style.css` `:root`.
3. ✅ **Hero + 7 sections** shipped as the complete site (see §5.2).
4. 🔲 **App icon** — still ships without a custom icon (mac `.app` declares one but bundles none; Windows exe likewise). Design once, ship on both.

---

*Companion: `product-plan.md` (positioning), `architecture.md` (runtime), `packaging.md` (distribution).*
