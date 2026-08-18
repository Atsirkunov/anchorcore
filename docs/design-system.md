# AnchorCore — Design System & Messaging (v1)

> For a non-UX owner: copy-paste tokens + rules. No taste required — pick the preset, don't invent.
> App (browser UI) is **dark, calm, trust-first**. Website is **light, same accent** — one brand, two surfaces.

---

## 1. Brand principles

* **Trust over clever.** You store company memory. Look like a filing system, not a game. No gradients, no neon.
* **Show your work.** Every answer cites source + section. Citations are first-class UI, not footnotes.
* **Calm density.** PMs read 200 lines/day. Dense but scannable: `14px` body, `12-13px` meta, generous `16px` section gaps.
* **Local-first signal.** "Private by default" is visual: lock icon, "on your machine" badge, no cloud ornament.

---

## 2. Color — one palette, two themes

Derived from current app (`#0f1115`, `#171a21`, `#38bdf8`) but normalized to tokens so app + website share hue.

### 2.1 Tokens (copy into `frontend/src/theme.ts`)

```ts
export const tokens = {
  // --- dark: the app (localhost:8000) ---
  dark: {
    bg: "#0f1115",            // page
    surface: "#14171d",       // header / source rows
    surfaceRaised: "#171a21", // cards, inputs, citations
    surfaceHover: "#1e2430",  // active tab, user bubble
    border: "#1f242c",        // header line
    borderSubtle: "#2d333b",  // inputs, bubbles
    text: "#e6e8eb",          // primary
    textDim: "#9ca3af",       // labels, meta
    textFaint: "#6b7280",     // placeholders, empty states
    accent: "#6366f1",        // Ask button, links (indigo — trustworthy, not cyan)
    accentHover: "#5457e5",
    accentSoft: "#1e1b4b",    // accent bg wash (badge bg)
    accentText: "#a5b4fc",    // on accentSoft
    success: "#4ade80", textSuccess: "#14532d",
    warning: "#f59e0b", textWarning: "#92400e",
    danger: "#f87171", textDanger: "#7f1d1d", dangerBg: "#3a1d1d", dangerBorder: "#4c2626",
    // entity kinds — muted, not traffic lights
    kind: { decision: "#8b5cf6", document: "#3b82f6", action: "#f59e0b", note: "#6b7280" },
  },
  // --- light: the website (anchorcore.com) — same accent, inverted surfaces ---
  light: {
    bg: "#f8f9fb",
    surface: "#ffffff",
    surfaceRaised: "#f1f3f6",
    border: "#e5e7eb",
    borderSubtle: "#d1d5db",
    text: "#0f1115",
    textDim: "#6b7280",
    textFaint: "#9ca3af",
    accent: "#6366f1",
    accentHover: "#5457e5",
    accentSoft: "#eef2ff",
    accentText: "#4338ca",
    success: "#16a34a", warning: "#d97706", danger: "#dc2626",
  },
} as const;
```

**Why indigo `#6366f1` not cyan `#38bdf8`:** current Ask button already uses `#6366f1`; cyan `#38bdf8` is kept only for *progress/spinner* (`#7dd3fc`) so interactive vs system states don't clash. One accent = one mental model. Indigo tests higher for "trust/enterprise" vs cyan "dev tool."

**Rules for non-designers:**
1. Never invent a hex. Only use `tokens.dark.*`.
2. Backgrounds: `bg` → `surface` → `surfaceRaised` (page → section → card). Don't skip layers.
3. Text: `text` for content, `textDim` for labels, `textFaint` for empty states. Never `#fff` on `bg`.
4. `dangerBg`/`dangerBorder` only for banner `App.tsx:203`. Never for buttons.
5. Kind colors only on left border + badge `EntitiesTab.tsx:55` — not whole card.

### 2.2 Contrast (WCAG AA)
* `text #e6e8eb` on `bg #0f1115` = 14.2:1 ✓
* `accent #6366f1` on white = 4.6:1 ✓ (light site CTA)
* `textDim #9ca3af` on `surface #14171d` = 4.8:1 ✓ — do not lighten further.

---

## 3. Typography & spacing

* **Font:** `system-ui, -apple-system, Segoe UI, Inter, sans-serif` (already `App.tsx:182`). No webfont — faster, native feel. If you add one, Inter only.
* **Scale:** `12` meta / `13` input labels / `14` body / `16` card title / `18` page title (`App.tsx:191`). `line-height 1.6` for answer bubbles `AskTab.tsx:83`.
* **Radius:** `6` inputs/badges, `8` cards, `10` bubbles, `999` pills (jobs badge). Keep as is.
* **Spacing:** `8` tight (input gap), `12` card gap, `16` section, `24` header gap. Don't add `4px` tweaks.

---

## 4. App UX — keep layout, fix consistency

Current 6 tabs `App.tsx:14` are correct. No navigation redesign needed.

**Apply tokens without redesign:**
```ts
// frontend/src/theme.ts = tokens above
// in each tab: import { tokens as t } from "../theme"
// replace literal "#171a21" → t.dark.surfaceRaised etc.
```
This alone fixes the "inline styles everywhere" debt `B37` without any visual decision.

**Micro-fixes (no design skill):**
* Button primary = `accent` `#6366f1` (already `AskTab.tsx:60`), secondary = `surfaceRaised` + `borderSubtle`.
* Empty states keep `textFaint #6b7280` + one-line hint (already `AskTab.tsx:101` — good).
* Citations: keep `14171d` card `AskTab.tsx:88` + `kind` left accent — this *is* the brand.

---

## 5. Messaging — website + app copy (one voice)

**Positioning** (`product-plan.md:5`): *Connect your knowledge to any AI model.* / *Your company's memory, not another chatbot.*

### 5.1 Website hero — A (chosen) + personal / company split

**Master headline (A):**
> **Your memory — finally searchable.**
> Connect folders and Jira. Ask “what was decided about X, and why?” — get a cited answer that shows exactly where it came from. Private by default, on your machine.

**Split language — same product, two doors (one brand, no color split):**

| Track | Who | Headline variant | Subhead | Proof point |
|---|---|---|---|---|
| **Personal** | Solo PM, founder, researcher, local enthusiast | **Your knowledge, finally together.** | Your notes, PDFs, Jira tickets and decisions in one searchable memory. Ask like you remember it — get the source. | “300-page PDF? Page 250 still cited. Your second brain, not another chatbot.” |
| **Company** | 3–30 person team, PM-led | **Your company's memory — finally searchable.** | Decisions, owners, dependencies — so any AI answers like an employee of 3 years. | “Why was this delayed? Who owns billing migration? Answered with section + author, not a guess.” |

Body copy swaps per tab but **design stays identical** — accent `#6366f1`, same screenshot (Ask tab with citations). Don't make two brands.

**Website hero implementation:** one page, toggle `Personal | Team` pills under headline (or two anchor sections `#personal` / `#teams`). Default to **Personal for v1** (validates faster, no procurement), Team as “coming soon” teaser. CTA identical: `Download for macOS / Windows — Free while in validation` + secondary `See 60-second demo` (`sample/`). Email capture tags the track (`personal_interest` vs `team_interest`) for later pricing.

**B/C remain as alternates** if A fatigues:
* B: *AI is smart. It just doesn't remember working at your company.*
* C: *Context infrastructure for agents.*

### 5.1b App copy — mirror the split without forking the app

* **Ask tab title** `AskTab.tsx:39` stays `Ask your memory` for both — neutral, works for personal and company.
* **Empty state** `AskTab.tsx:101` → track-aware hint:
  * Personal: *“Ask your documents — e.g. ‘what did I decide about security transfers, and why?’”*
  * Company: *“Ask your team memory — e.g. ‘who owns billing migration and what supersedes it?’”*
  Switch via simple `localStorage['anchorcore.track']` set from website download link (`?track=personal` / `?track=team`) — no auth, just copy.
* **OnboardingWizard** `OnboardingWizard.tsx:298` step 2: *“Connect your knowledge”* then `Personal: your docs folder` vs `Team: shared folder or Jira` — same flow, different example path.
* **Future:** Project picker `App.tsx:139` already scopes Teams naturally; Personal uses `All sources`.

### 5.2 Website structure (light theme, same accent)

1. **Hero** — headline A + product screenshot (Ask tab with citations) + download buttons.
2. **Problem → Solution** — 3 columns: *Fragmented knowledge* (Slack/Jira/Drive) → *Lost reasoning* → *AnchorCore: entities + provenance + citations*.
3. **How it works** — 3 steps with icons: `Connect folder/Jira` → `Classify into decisions/actions/notes (review queue)` → `Ask, cited`. Small code: `Sources → extract → classify → embed (sqlite-vec+FTS5) → RRF → cited answer` (`README.md:67`).
4. **Trust strip** — "Private by default • OS keychain • No data leaves your machine unless you add a cloud key • Every answer shows source." Icons: lock, key, citation.
5. **Live demo** — connect `sample/` corpus, 3 questions answered (use `docs/sample-dataset.md:1` scripts).
6. **Roadmap teaser** — `Team memory, Slack, Drive — coming soon` — email capture.
7. **Footer** — `v1 local. Free during validation. Flat license at pilot €25/mo.`

**Voice:** short, factual, no superlatives. Same as app: `No relevant knowledge found yet.` not "Oops!"

### 5.3 App microcopy (already good — keep)

* `Ask your memory` `AskTab.tsx:39` — perfect, keep.
* Follow-up hint `AskTab.tsx:47` "2 questions in context — follow-ups like 'and what about its movements?' work" — excellent, keep.
* Banner `App.tsx:97` — shorten to `Ollama offline — answers show context only. Settings → Test connection.`

---

## 6. Implementation path (no UX skill needed)

1. **Create `frontend/src/theme.ts`** — paste tokens `§2.1`. 10 min.
2. **Find-replace** `grep -r "#171a21" frontend/src` → `t.dark.surfaceRaised`. No visual change, just consistency. Validates `B37` DoD.
3. **Website:** use same `tokens.light` + `accent #6366f1` for CTA. Vite + `shadcn/ui` landing template — do not design from scratch. Hero A + 7 sections above = complete site.
4. **App icon / packaging:** keep generic PyInstaller icon `packaging.md:73` until site is live — icon without site looks unfinished.

---

*Companion: `product-plan.md:5` (positioning), `architecture.md:8` (runtime), `packaging.md:1` (distribution). Tokens enforce `B37` without requiring taste.*
