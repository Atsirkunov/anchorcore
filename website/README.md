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

* Hero copy is authored once in `index.html` (personal only). The Personal/Team/Hosted hero toggle and its `copy` swap were retired in R44.16: Team and Hosted have their own pages (below). Old `?track=team|hosted` links redirect client-side to `/team` / `/hosted` (`redirectLegacyTrack` in `src/main.ts`); `?track=personal` is a no-op.
* Pages (each a Vite entry in `vite.config.ts`, clean URL via `html_handling`): `index.html` `/`, `privacy.html` `/privacy`, `team.html` `/team` (Personal-vs-Team table, differentiators, quote form), `hosted.html` `/hosted` (one-field waitlist). Team/Hosted share `src/pages.ts`; header/footer are copied from `index.html`, so edit all three when nav changes. New pages go in `public/sitemap.xml` too.
* Team page claims must stay checkable in the repo: label gate (`answer.rs`, `docs/gated-access-spec.md`), `system_events` gate records, MCP `public_only` + audit (`docs/mcp.md`), uid backup (`docs/backup.md`), one-container image (`hosting/README.md`). No price, range or fee structure anywhere; no SSO/roles claims before R43.18; no "Built by" angle.
* `index.html` sections (`Hero`, `Problem→Solution`, `How it works`, `Demo`, `Agent skill`, `Guides`, `FAQ`, `Pricing`, `Get started`, `Privacy`, `Footer`) — all in one file, no component abstraction. `Guides` holds both the entry cards and the guide reader (R18.7 merged the former `Guidebook` section into it). The landing examples use the `sample/` payments corpus only; the `?corpus=ai` swap was dropped when the hero became a simulation (R44.11), and `sample-ai/` survives as the "AI startup" guide tab.
* Two-path strip under the hero (agent card) + `#agent-skill` both carry `npx skills add Atsirkunov/anchorcore --skill anchorcore-setup` — keep the two in sync if the CLI syntax changes (verified against `npx skills add Atsirkunov/anchorcore --list`: the repo exposes `anchorcore-setup` and `aida-landing-page`)
* `src/style.css` `:root` tokens — change `--sage` or `--paper` once
* Header nav exists twice on purpose — `.nav` (desktop, ≥1081px) and the `.navmenu` `<details>` disclosure (≤1080px, works without JS); edit both when adding a section link (R18.1/R18.3)
* The widescreen scale lives in `:root` clamps — `--wrap`, `--wrap-narrow`, `--fs-h1`, `--fs-h2`, `--fs-body`, `--fs-h3`, `--pad-sec` are the identity value up to ~1500px and grow on 2K/4K (R18.16); change the clamp, not the rule
* Examples (hero mock, terminal, graph, `public/*.svg`) come from the repo's `sample/` payments corpus — keep quotes matching those files; never reuse `business-scale` corpus terms (e.g. `ACME`) in user-facing copy

## Wording glossary (R44.9)

* Category noun: **private AI search** (the product is "a private AI search app"); the output is a **cited answer**. Those are the only category terms on the landing. Retired from landing copy: "memory", "second brain", "knowledge base" ("company memory" may stay on Team-track copy only).
* One CTA label per action, identical everywhere (hero, header, pricing, get-started): **Download — free for personal use** / **Watch it work** (links `#demo`) / **Join the waitlist** (also the `/hosted` form button). The OS-specific text lives in the note under the button, not in the label. Team's action is **Request a quote** (pricing card and `/team` form; links `/team`). Phones keep "Email me the download link".
* "Any JSON API" is a capability statement only: JSON list endpoints with field mapping (no GraphQL/XML/browser-OAuth flows), linking to the REST guide. Never add safety wording ("safe", "sandboxed", "protected") about it: the REST connector has no SSRF/private-address guard.

## Deploy

Live at anchorcore.dev via Workers Static Assets (`wrangler.jsonc`: `anchorcore-website` worker, `anchorcore.dev/*` route + custom hostname; CI auto-deploys on `v*` tags when `website/` changed since the previous tag — `release.yml` `deploy-website`, secrets set 2026-10-11; manual fallback: `wrangler login` once, then `npm run build && npx wrangler deploy`). Deploy only from a committed tree — live must never be ahead of `main` (guides are imported from `docs/` at build time, so uncommitted doc edits ship silently). `Download` buttons link straight at the latest release assets (`.../releases/latest/download/AnchorCore-{windows,macos}.zip`, OS-aware via `src/main.ts`); live since `v1.0.13`. Keep the `release.yml` asset names stable.

## Social / brand assets (R16.5)

* `public/favicon.svg` is the source of truth for the citation-bar mark (tab icon, header, footer, `frontend/public/` copy). SVG favicons work natively — no PNG needed.
* `public/og-card.svg` backs the `og:image` / `twitter:image` tags. Strict crawlers (X, iMessage) want PNG: export once per deploy with `rsvg-convert -w 1200 public/og-card.svg -o public/og-card.png` and point the tags at the absolute deploy URL (`og:image` must be absolute — live at `https://anchorcore.dev/og-card.png`).

## Capture

Forms POST to Formspree (`FORMSPREE_ID` in `src/capture.ts`) as JSON `{email, interest, source}`. `/team` adds `company`, `seats`, `deployment`, `note` (`interest=team`, `source=team-page`); `/hosted` sends `interest=hosted`, `source=hosted-page`; the landing form's interest comes from the `data-interest` link that led to it.

## Privacy & GDPR

No cookies, no analytics, no third-party requests on page load — so no cookie
banner needed. The Art. 13 notice is its own page: `privacy.html` is a second
Vite entry and builds to `/privacy` (R18.11), while `index.html` keeps a
two-line summary in `#privacy` — the nav still links there, so its label keeps
matching the on-page heading (R18.3). Keep both in sync with `src/capture.ts`.
Only data flow: waitlist and quote forms → Formspree
(owner must conclude the Art. 28 DPA in their Formspree account). No browser
storage since the hero toggle was retired. Do not add analytics/trackers without revisiting the notice + consent.
****Edge-injected agent bridge (R18.10).** Cloudflare's WebMCP preview adds
`/.webmcp/bridge.js` to every HTML response for this domain. It is **not a repo
asset** — `git grep webmcp` finds only the disclosures — and it is served from
our own origin, so it is not a third-party request. Toggle: Cloudflare
dashboard → Agent Readiness → WebMCP, where the default packs "Content
Credentials" (C2PA) and "Site MCP Server" are what `data-packs="c2pa,mcp-server-client"`
renders. Audited 2026-10-11 from the served bytes (73 KB, 1 file, minified): no
external URLs, no cookies or storage, no XHR/WebSocket/sendBeacon/postMessage;
it registers `scan_images_c2pa` / `inspect_image_c2pa` plus the site-as-MCP-server
bridge, and the image tool fetches images already on the page when an agent calls
it. Turning it off is a dashboard toggle — nothing in the repo changes. The
privacy notice discloses it (keep that line in sync if the packs change).
Impressum: deferred, not forgotten.** §5 DDG requires a publishable postal
address for a commercial site; the owner has not supplied one, and shipping a
placeholder would be worse than the deferral. When it exists, add the block to
`privacy.html` (see the comment in its `<head>`).
