# AnchorCore — Design System & Messaging

> For a non-UX owner: copy-paste tokens + rules. No taste required — pick the preset, don't invent.
> App and website share one look (**Archive**, 2026-10-22, tuned by R49.7 on 2026-11-02): ink on paper, IBM Plex, oxblood for citations only. The app is light by default with a matching cool-black dark set, chosen by the header Auto / Light / Dark switch (Auto follows the OS; the switch moves to Settings → Appearance with R49.9). The website opens in Light, with Light / Auto / Dark in the footer.

---

## 1. Brand principles

* **Trust over clever.** You store company memory. Look like a filed document, not a game. No gradients, no glow, no emoji.
* **Show your work.** Every answer cites source + section. Citations are first-class UI, not footnotes — and the only thing that gets the accent colour.
* **Calm density.** Dense but scannable: `14px` body, `12-13px` meta, `16px` serif answers, generous section gaps.
* **Local-first signal.** "Private by default" is stated in words where it matters, not decorated.

---

## 2. Color — one palette, light + dark

Olive (R49.12–R49.14, owner decision 2026-11-02): olive-tinted ink `#23251A`
on olive-cream paper `#F2F1E8`. One rule carries the colour: **olive is the
surface, oxblood means "sourced", green means "healthy".** Oxblood `#8C2F2B`
marks citations: citation marks, source ids, the active citation card and the
brand mark, nothing else. The status green is teal-leaning (`#23604F`) so
"Synced/Saved/up" never blends into the olive surfaces. On the site the hero's
search path travels in olive (`--nx-path`) and only the hit turns oxblood.
Links are ink + underline, oxblood on hover. Primary buttons are ink, never
oxblood (red reads as "delete" on a button).

### 2.1 Tokens (single sources of truth)

App tokens are CSS variables in `frontend/src/theme.css` (`--ac-*`, light in
`:root`, dark under `:root[data-theme="dark"]`, set by `themeMode.ts` + an inline
pre-paint script in `index.html`; the choice is stored as `theme-mode` in localStorage); components read them through
`frontend/src/theme.ts` (`theme.*` = `var(--ac-*)` strings). Site tokens live in
`website/src/style.css` (`:root`).

| Role | App light (`--ac-*`) | App dark | Site (`style.css`) |
|---|---|---|---|
| page bg | `bg #F2F1E8` | `#181A12` | `--paper #F2F1E8` |
| card / panel | `bg-card #FAFAF4`, `bg-elevated #FAFAF4` | `#22251A`, `#1D2016` | `--surface #FAFAF4` |
| borders | `border #D6D4C2`, `border-soft #E3E2D3` | `#383C2B`, `#2A2E20` | `--border #D6D4C2` |
| text | `text #23251A`, `text-muted #45473A`, `text-dim #64674C` | `#EEEEE4`, `#C6C8B0`, `#A3A58C` | `--ink`, `--dim #55583F` |
| primary button | `accent #23251A` on `#FAFAF4` | `#EEEEE4` on `#12140D` | `--ink` on `--on-ink` |
| citations (links are ink) | `cite #8C2F2B`, `cite-soft #F1E4DC` | `#CC8A80`, `#36261F` | `--cite #8C2F2B`, `--cite-soft` |
| status | `green #23604F`, `amber #85550F`, `red #A3332C`, `blue #2F5577`, `slate #4E5B69` | lighter tints (`green #84C4B0`) | `--term-ok` |
| hero search path | (n/a) | (n/a) | `--nx-path #5A6B23` / dark `#B5C47A` |
| type | `--ac-sans` / `--ac-serif` / `--ac-mono` = IBM Plex Sans / Serif / Mono | same | `--sans` / `--serif` / `--mono` |

**Rules for non-designers:**
1. Never invent a hex. App: only `theme.*` (add a token to `theme.css` + `theme.ts` first). Site: only `:root` vars.
2. Oxblood only for citations (marks, source ids, the active citation card) and the brand mark. Links, selected state, step numbers and labels are ink or dim. Errors use `red`, which is brighter.
3. No coloured left-border stripes on cards; say the status in words (a coloured label).
4. Danger bg/border only for banners and destructive buttons.
5. When changing a shared hue, change `theme.css` and `website/src/style.css` in the same commit.

### 2.2 Contrast (WCAG AA)
* App: lowest text on card 5.6:1 light / 6.2:1 dark ✓ · `cite` on card 7.8:1 / 5.6:1 ✓ · `text-dim` on hover 4.5:1 / 5.4:1 ✓
* Site: `--nx-path` on paper 5.2:1 / 9.3:1 · `--cite` on paper 7.3:1 / 6.3:1 · button text 14.8:1 / 15.9:1
* Check every pair before changing a token: `python3 scripts/check_contrast.py frontend/src/theme.css` (text, status and cite on every surface, both modes; AA 4.5:1).

---

## 3. Typography & spacing

* **Fonts:** IBM Plex, bundled with the app and the site (`@fontsource/*`, latin subsets) — the app works offline and the site loads nothing from third parties. Sans for UI, labels and badges; Serif for headings, questions and answer text; Mono only for code, paths, line ranges, record ids and citation marks (R44.14, R49.7).
* **Scale:** `11` labels / `12` meta / `14` body / `16` serif answer / `20` question / page titles serif.
* **Radius:** `radius = 2` from `theme.ts` everywhere; `50%` only for dots and spinners.
* **Spacing:** `8` tight, `12` card gap, `16-28` between answers/sections.

---

## 4. App UX

Five tabs, each with a hash route (R49.1, R49.9): Ask · Library (list first, `#/library/<id>` per source) · Records (decisions, actions) · Agents (MCP + skills + the agents switch) · Settings on the right (rail: Answers · Search · Sensitive sharing · Glossary · Backup & restore · Updates · Status & logs · Appearance · Advanced). Header: brand mark, tabs with an ink underline on the active one, one ink rule under the bar; a status dot replaces the "API online" pill (R49.3).

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
7. **Privacy notice + Footer** — GDPR notice section; `AnchorCore is free for personal use · Team self-hosted on request · Hosted soon.` Light / Auto / Dark switch at the end.

**Voice:** short, factual, no superlatives. Same as app: `No relevant knowledge found yet.` not "Oops!"

### 5.3 App microcopy (already good — keep)

* `Ask your memory` — perfect, keep.
* Follow-up hint ("N questions in context — follow-ups work") — excellent, keep.
* Model health (`modelHealth.ts`, R49.3): `Ollama is offline — …` followed by what actually degrades for the current mode (search keyword-only, local answers basic) — keep it mode-aware. It is the header status dot's tooltip, not a banner; Ask adds one dismissible line when no answer model works. Banners are for failures (a source whose sync keeps failing).

---

## 6. Implementation record

1. ✅ **`frontend/src/theme.ts`** created — all app color through tokens, zero literal hex elsewhere.
2. ✅ **Website** uses the same palette via `website/src/style.css` `:root`.
3. ✅ **Hero + 7 sections** shipped as the complete site (see §5.2).
4. 🔲 **App icon** — still ships without a custom icon (mac `.app` declares one but bundles none; Windows exe likewise). Design once, ship on both.

---

*Companion: `product-plan.md` (positioning), `architecture.md` (runtime), `packaging.md` (distribution).*
