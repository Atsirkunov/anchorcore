# AnchorCore — Design System & Messaging

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

Stone & Sage (C3) — vault/archive, matte, non-AI. Dark app = warm black
`#121416` + sage `#4A5A52`; light site = paper `#F2F0EB` + same sage.
Replaces indigo/cyan to signal *memory / filing system*, not chatbot.

### 2.1 Tokens (single sources of truth)

App tokens live in `frontend/src/theme.ts` (`theme.*`); site tokens in
`website/src/style.css` (`:root`). Same hues, different key names:

| Hue | App (`theme.ts`) | Site (`style.css`) |
|---|---|---|
| page bg | `bg #121416` | `--paper #F2F0EB` |
| surface / card | `bgElevated #1A1E20`, `bgCard #23282B` | `--surface #ffffff` |
| borders | `border #343a3e`, `borderSoft #2a2f33` | `--border #e2ddd6`, `--border2 #d4cfc6` |
| text | `text #E8E6E1`, `textMuted #9aa0a8`, `textDim #7a828c` | `--ink #1a1d1a`, `--dim #6b7280`, `--faint #9aa0a8` |
| accent (sage) | `accent #4A5A52`, `accentAlt #8FA99E` | `--sage #4A5A52`, `--sageH #3f4d46`, `--sageSoft #E2E8E3` |
| stone | `amber #9A8B7A` | `--stone #9A8B7A` |
| kinds | `purple #6E7D75`, `blue #7E9AB0`, `amber`, `green #7a9a8a` | (n/a — app only) |
| danger | `red #c98a7a`, `redBg #2B1E1D`, `redBorder #3d2a28` | `#a85a4a` |

**Why sage `#4A5A52` not indigo `#6366f1`:** indigo tested as trustworthy but reads as chatbot/SaaS (Linear/Notion AI). Sage is desaturated, editorial — signals *archive / foundry / vault* = memory + reliability + privacy. Progress/spinner uses the sage tint, not cyan. One matte accent = one mental model.

**Rules for non-designers:**
1. Never invent a hex. App: only `theme.*`. Site: only `:root` vars.
2. Backgrounds layer page → section → card. Don't skip layers.
3. Text: content → labels → empty states, brightest to faintest. Never pure white on dark bg.
4. Danger bg/border only for banners, never for buttons.
5. Kind colors only on left border + badge — not whole cards.
6. When changing a shared hue, change `theme.ts` and `style.css` in the same commit.

### 2.2 Contrast (WCAG AA) — Stone & Sage
* `text #E8E6E1` on `bg #121416` = 13.9:1 ✓
* `accent #4A5A52` on paper `#F2F0EB` = 6.8:1 ✓ (light site CTA, matte)
* `accent #4A5A52` on white = 7.2:1 ✓
* `textMuted #9aa0a8` on dark surface = 4.9:1 ✓ — do not lighten further.

---

## 3. Typography & spacing

* **Font:** `system-ui, -apple-system, Segoe UI, Inter, sans-serif`. No webfont — faster, native feel. If you add one, Inter only. (Site hero uses a Georgia serif for the non-AI editorial tell.)
* **Scale:** `12` meta / `13` input labels / `14` body / `16` card title / `18` page title. `line-height 1.6` for answer bubbles.
* **Radius:** `6` inputs/badges, `7-8` cards, `8` bubbles, `999` pills. Matte, no glow.
* **Spacing:** `8` tight (input gap), `12` card gap, `16` section, `24` header gap. Don't add `4px` tweaks.

---

## 4. App UX — keep layout, fix consistency

Current 7 tabs (Ask, Sources, Entities, Review, PII, Settings, System) are correct. No navigation redesign needed.

**Token adoption: done.** `frontend/src/theme.ts` is the single source and no
literal hex remains outside it. New UI must import `theme` / `commonStyles`
instead of inlining colors.

**Standing micro-rules:**
* Button primary = sage accent, secondary = card bg + subtle border.
* Empty states keep faint text + one-line hint.
* Citations: dark card + kind-colored left accent — this *is* the brand.

---

## 5. Messaging — website + app copy (one voice)

**Positioning:** *Your memory — finally searchable.* / *Connect your knowledge to any AI model.*

### 5.1 Website hero — Personal / Team / Hosted tracks

**Master headline:**
> **Your memory — finally searchable.**
> Turn folders, Drive, Jira and Linear into a private AI knowledge base. Ask “what was decided about X, and why?” — get a cited answer that shows exactly where it came from. Private by default, on your machine.

**Track copy (same product, three doors — one brand, no color split):**

| Track | Who | Headline variant | Subhead | Proof point |
|---|---|---|---|---|
| **Personal** | Solo PM, founder, researcher | **Your memory — finally searchable.** | Your notes, PDFs, Jira, Linear and Drive in one searchable private memory. Ask like you remember it — get the source. | “300-page PDF? Page 250 still cited. Your second brain, not another chatbot.” |
| **Team** | 3–30 person team, PM-led | **Run on your infrastructure — one container.** | Self-hosted knowledge base in one command. Your data stays with you, same cited answers for the whole team. One-time platform license, yours to run — no cloud required. | “One Docker container — your team's memory, on your infra. Who owns billing? Cited, not guessed.” |
| **Hosted** | Waiting list | **Hosted — coming soon.** | We host it for you, same private memory, no setup. Join waitlist for early access. | “No Docker, no infra — just connect and ask. Your memory, hosted.” |

Body copy swaps per tab but **design stays identical** — same accent, same
cited-answer mock. Don't make two brands.

**Implementation (shipped):** one page, `Personal | Team · Self-hosted |
Hosted` toggle under the headline; track persisted to site `localStorage` +
`?track=` param. Default **Personal**. CTA per track: Personal = OS-aware
download button, Team = Docker/hosting docs link, Hosted = waitlist anchor.
Email capture tags the track. No prices on site.

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

1. **Hero** — headline + cited-answer mock + download buttons.
2. **Problem → Solution** — 3 columns: *Fragmented knowledge* → *Lost reasoning* → *AnchorCore: every answer shows its source*.
3. **How it works** — 3 steps with icons + plain pipeline line (`connect → read → file → ask → cited answer`), sections visual, relationship visual, sealed-by-design privacy strip.
4. **Live demo** — connect `sample/`, 4 questions answered (scripts in `docs/sample-dataset.md`); browser + MCP columns; abbreviated agent-session replay.
5. **Guides + Guidebook** — folder/projects/PII/REST cards, Obsidian spotlight, one-at-a-time guide panels rendered from repo markdown.
6. **FAQ, Pricing, Roadmap** — 7 FAQs, 3 tiers without prices, download + waitlist capture.
7. **Privacy notice + Footer** — GDPR notice section; `v1 local · free for personal use · Team self-hosted licensed · Hosted soon.`

**Voice:** short, factual, no superlatives. Same as app: `No relevant knowledge found yet.` not "Oops!"

### 5.3 App microcopy (already good — keep)

* `Ask your memory` — perfect, keep.
* Follow-up hint ("N questions in context — follow-ups work") — excellent, keep.
* Offline banner: `Ollama offline — classification falls back to rules. Settings → Test connection.` — current text, keep.

---

## 6. Implementation record

1. ✅ **`frontend/src/theme.ts`** created — all app color through tokens, zero literal hex elsewhere.
2. ✅ **Website** uses the same palette via `website/src/style.css` `:root`.
3. ✅ **Hero + 7 sections** shipped as the complete site (see §5.2).
4. 🔲 **App icon** — still ships without a custom icon (mac `.app` declares one but bundles none; Windows exe likewise). Design once, ship on both.

---

*Companion: `product-plan.md` (positioning), `architecture.md` (runtime), `packaging.md` (distribution).*
