# AnchorCore Website — tweakable single page

> Copy-paste site, no taste required. Edit `index.html` or `src/main.ts` `copy` object, not CSS.

Stack: Vite static, `Stone & Sage` `paper #F2F0EB` + `sage #4A5A52` same tokens as `frontend/src/theme.ts` / `docs/design-system.md:17`.

## Run

```bash
cd website
npm install
npm run dev    # http://localhost:5174
npm run build && npx wrangler deploy  # https://anchorcore.dev
```

## Tweakable copy

* `src/main.ts` `copy.personal` / `copy.team` — hero title/sub/proof + `?track=personal|team` + `localStorage["anchorcore.track"]` (track-aware app copy is designed but not wired — `design-system.md` §5.1b)
* `index.html` sections (`Hero`, `Problem→Solution`, `How it works`, `Demo`, `Guides`, `Guidebook`, `FAQ`, `Pricing`, `Roadmap`, `Privacy`, `Footer`) — all in one file, no component abstraction
* `src/style.css` `:root` tokens — change `--sage` or `--paper` once
* Header nav exists twice on purpose — `.nav` (desktop, ≥761px) and the `.navmenu` `<details>` disclosure (≤760px, works without JS); edit both when adding a section link (R18.1)
* The widescreen scale lives in `:root` clamps — `--wrap`, `--wrap-narrow`, `--fs-h1`, `--fs-h2`, `--fs-body`, `--fs-h3`, `--pad-sec` are the identity value up to ~1500px and grow on 2K/4K (R18.16); change the clamp, not the rule
* Examples (hero mock, terminal, graph, `public/*.svg`) come from the repo's `sample/` payments corpus — keep quotes matching those files; never reuse `business-scale` corpus terms (e.g. `DVCA`) in user-facing copy

## Deploy

Live at anchorcore.dev via Workers Static Assets (`wrangler.jsonc`: `anchorcore-website` worker, `anchorcore.dev/*` route + custom hostname; `wrangler login` once, then `npm run build && npx wrangler deploy`). Deploy only from a committed tree — live must never be ahead of `main` (guides are imported from `docs/` at build time, so uncommitted doc edits ship silently). `Download` buttons link straight at the latest release assets (`.../releases/latest/download/AnchorCore-{windows,macos}.zip`, OS-aware via `src/main.ts`); live since `v1.0.13`. Keep the `release.yml` asset names stable.

## Social / brand assets (R16.5)

* `public/favicon.svg` is the source of truth for the citation-bar mark (tab icon, header, footer, `frontend/public/` copy). SVG favicons work natively — no PNG needed.
* `public/og-card.svg` backs the `og:image` / `twitter:image` tags. Strict crawlers (X, iMessage) want PNG: export once per deploy with `rsvg-convert -w 1200 public/og-card.svg -o public/og-card.png` and point the tags at the absolute deploy URL (`og:image` must be absolute — live at `https://anchorcore.dev/og-card.png`).

## Capture

Forms POST to Formspree (`FORMSPREE_ID` in `src/main.ts`) with the email + current track + source.

## Privacy & GDPR

No cookies, no analytics, no third-party requests on page load — so no cookie
banner needed. The `#privacy` section in `index.html` is the Art. 13 notice;
keep it in sync with `src/main.ts`. Only data flow: waitlist form → Formspree
(owner must conclude the Art. 28 DPA in their Formspree account). Only storage:
`localStorage["anchorcore.track"]` hero-tab preference (exempt, disclosed in the
notice). Do not add analytics/trackers without revisiting the notice + consent.
Impressum still pending a publishable postal address (see pre-launch notes).
