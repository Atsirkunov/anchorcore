# AnchorCore Website — tweakable single page

> Copy-paste site, no taste required. Edit `index.html` or `src/main.ts` `copy` object, not CSS.

Stack: Vite static, `Stone & Sage` `paper #F2F0EB` + `sage #4A5A52` same tokens as `frontend/src/theme.ts` / `docs/design-system.md:17`.

## Run

```bash
cd website
npm install
npm run dev    # http://localhost:5174
npm run build  # dist/ -> deploy to GitHub Pages / Cloudflare / Vercel
```

## Tweakable copy

* `src/main.ts` `copy.personal` / `copy.team` — hero title/sub/proof + `?track=personal|team` + `localStorage["anchorcore.track"]` for app hint `AskTab.tsx:39`
* `index.html` hero, 7 sections (`Hero`, `Problem→Solution`, `How it works`, `Trust strip`, `Live demo`, `Roadmap`, `Footer`) — all in one file, no component abstraction
* `src/style.css` `:root` tokens — change `--sage` or `--paper` once

## Deploy

Any static host. `Download` links currently go to `https://github.com/Atsirkunov/anchorcore/releases` — replace with direct asset URLs after `v1.0.11` tag.

## Social / brand assets (R16.5)

* `public/favicon.svg` is the source of truth for the citation-bar mark (tab icon, header, footer, `frontend/public/` copy). SVG favicons work natively — no PNG needed.
* `public/og-card.svg` backs the `og:image` / `twitter:image` tags. Strict crawlers (X, iMessage) want PNG: export once per deploy with `rsvg-convert -w 1200 public/og-card.svg -o public/og-card.png` and point the tags at the absolute deploy URL (`og:image` must be absolute — fill in the domain when it lands).

## Capture

Forms store `localStorage["anchorcore.capture:personal|team"]` — hook to `POST /api/capture` or email service when ready.
