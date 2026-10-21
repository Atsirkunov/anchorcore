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
* Pages (each a Vite entry in `vite.config.ts`, clean URL via `html_handling`): `index.html` `/`, `privacy.html` `/privacy`, `team.html` `/team` (Personal-vs-Team table, differentiators, quote form), `hosted.html` `/hosted` (one-field waitlist). Team/Hosted share `src/pages.ts`; header/footer are copied from `index.html`, so edit all three when nav changes. `sitemap.xml` is generated at build by the `anchorcore-sitemap` plugin in `vite.config.ts` (static page list there + every guide in `scripts/guides.json`); there is no committed sitemap, so add a new static page to `STATIC_PAGES` in `vite.config.ts`.
* Team page claims must stay checkable in the repo: label gate (`answer.rs`, `docs/gated-access-spec.md`), `system_events` gate records, MCP `public_only` + audit (`docs/mcp.md`), uid backup (`docs/backup.md`), one-container image (`hosting/README.md`). No price, range or fee structure anywhere; no SSO/roles claims before R43.18; no "Built by" angle.
* `index.html` sections (`Hero`, `Problem→Solution`, `How it works`, `Demo`, `Agent skill`, `Guides`, `FAQ`, `Pricing`, `Get started`, `Privacy`, `Footer`) — all in one file, no component abstraction. `Guides` holds the entry cards only; each links to a prerendered page under `/guides/` (R44.15, below). The landing examples use the `sample/` payments corpus only; the `?corpus=ai` swap was dropped when the hero became a simulation (R44.11), and `sample-ai/` survives as the "AI startup" guide page.
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

## Guides (R44.15)

Guides are static pages rendered at build time from the repo docs, so crawlers without JS (incl. AI answer engines) see the full text. `marked` runs only in `scripts/build-guides.mjs` (devDependency); the client bundle has no markdown code.

* `scripts/guides.json` is the single list: `slug`, `source` (repo-relative `.md`), `title`, `description`. It feeds the generator, the Vite entries and the sitemap.
* `npm run guides` (run by `dev` and `build`) writes `guides/<slug>.html` (gitignored intermediates; never commit). Each page lifts the header and footer from `team.html` (Guides link marked current), so nav edits there flow through. Vite builds them to `dist/guides/<slug>.html`, which `html_handling: auto-trailing-slash` serves at `/guides/<slug>` (same mechanism as `/privacy`). There is no `/guides` index page.
* Safety: raw HTML in markdown is escaped, not passed through; `javascript:`/`data:` links are dropped. Relative links between docs become `/guides/<slug>` when the target is a listed guide, otherwise a GitHub blob/tree URL; `https://anchorcore.dev/...` links are made site-relative (`/sample.zip` ships from `public/`, built by `sample-zip`).
* Headings keep their markdown level (the H1 becomes the page `<h1>`), get GitHub-style ids and a `#` anchor link. Tables are wrapped for horizontal scroll on phones. Typography lives in `.guide-body` / `.guide-page` in `src/style.css`.
* Landing `#guides` cards link to the pages; old `#guide-obsidian|quickstart|rest|ai|mcp` links redirect client-side (`redirectLegacyGuide` in `src/main.ts`) and, without JS, land on the card with that id.
* **Add a guide:** write the doc, add an entry to `scripts/guides.json`, add a card in `index.html` (`id="guide-<slug-ish>"`), and add the id to `GUIDE_PAGES` in `main.ts` only if old links need it. Docs are rendered as-is: keep claims in the doc consistent with the site's rules (no pricing, no SSRF-guard claim, keychain only on macOS/Windows with an encrypted-file fallback, Windows/Linux not code-signed).

## Social / brand assets (R16.5)

* `public/favicon.svg` is the source of truth for the citation-bar mark (tab icon, header, footer, `frontend/public/` copy). SVG favicons work natively — no PNG needed.
* `public/og-card.svg` is the source for `public/og-card.png` (1200x630, what `og:image`/`twitter:image` point at, absolute URL `https://anchorcore.dev/og-card.png`). Regenerate the PNG whenever the SVG copy changes, e.g. with headless Chrome (no extra deps): `"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" --headless=new --hide-scrollbars --window-size=1200,630 --screenshot=public/og-card.png file://$PWD/public/og-card.svg`, or `rsvg-convert -w 1200 public/og-card.svg -o public/og-card.png`. Check the text is not clipped by the card.

## Structured data + social tags (R44.18)

`scripts/seo.ts` (the `anchorcore-seo` Vite plugin, `transformIndexHtml`, runs on every page) generates these at build so they cannot drift:

* **JSON-LD (landing only):** `FAQPage` is built from the visible `#faq` markup (`h3` = question, `p` = answer, tags stripped, entities decoded). Do not hand-write it; edit the visible FAQ. The build fails if the FAQ parses to zero entries, if parsed count differs from the number of `.qa` blocks, or if the emitted JSON-LD text differs from the visible text. `SoftwareApplication` takes `description` from the page meta description, `softwareVersion` from `rust/Cargo.toml` `workspace.package.version` (unreadable: build warns and omits it), `downloadUrl` = the releases URL of the hero CTA, `screenshot` = the og image, publisher Person (owner-provided LinkedIn `sameAs`). Offer price `0` is the free personal app only; no Team/Hosted offers, no claims beyond the page. If the hero CTA URL changes, update `RELEASES` in `seo.ts`.
* **Social tags (all pages):** the plugin adds any missing `og:site_name`, `og:title/description/url/type`, `og:image` (+ `width`/`height`/`alt`), `twitter:card=summary_large_image`, `twitter:title/description/image(+alt)` from each page's `<title>`, meta description and canonical (a page lacking one of those fails the build). Tags already written by hand or by `build-guides.mjs` win, so per-page titles live in one place (the page itself).
* `?track=` is no longer written to the URL anywhere (only read by `redirectLegacyTrack`).
* No Rich Results test is run in CI; validate manually after deploy at search.google.com/test/rich-results and validator.schema.org.

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
