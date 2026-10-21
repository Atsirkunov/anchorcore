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
* Pages (each a Vite entry in `vite.config.ts`, clean URL via `html_handling`): `index.html` `/`, `privacy.html` `/privacy`, `team.html` `/team` (Personal-vs-Team table, differentiators, quote form), `hosted.html` `/hosted` (one-field waitlist). `security.html` `/security` (long form of the landing `#trust`: network-call table, label gate, secrets, limits; every claim cites a repo file — re-verify against `rust/crates/anchorcore/src` when connectors/clients change) and `vs/notebooklm.html` `/vs/notebooklm` (comparison; NotebookLM facts come only from Google's help pages linked in the page, re-check them before editing; NotebookLM was renamed Gemini Notebook on 2026-07-16) are hand-written pages that copy the `team.html` header/footer and use the guide layout (`.guide-page`/`.guide-body`, `src/guide.ts`); both are in `STATIC_PAGES`. They are linked from the footer, not the nav (nav stays capped at 5 + Download), and the footer link row exists in `index.html`, `team.html`, `hosted.html` (guides lift `team.html`'s). `public/llms.txt` is a hand-kept page index: update it when pages change and keep it to claims the pages make. Team/Hosted share `src/pages.ts`; header/footer are copied from `index.html`, so edit all three when nav changes. `sitemap.xml` is generated at build by the `anchorcore-sitemap` plugin in `vite.config.ts` (static page list there + every guide in `scripts/guides.json`); there is no committed sitemap, so add a new static page to `STATIC_PAGES` in `vite.config.ts`.
* Team page claims must stay checkable in the repo: label gate (`answer.rs`, `docs/gated-access-spec.md`), `system_events` gate records, MCP `public_only` + audit (`docs/mcp.md`), uid backup (`docs/backup.md`), one-container image (`hosting/README.md`). No price, range or fee structure anywhere; no SSO/roles claims before R43.18; no "Built by" angle.
* `index.html` sections (`Hero`, `Problem→Solution`, `How it works`, `Demo`, `Agent skill`, `Guides`, `FAQ`, `Pricing`, `Get started`, `Privacy`, `Footer`) — all in one file, no component abstraction. `Guides` holds the entry cards only; each links to a prerendered page under `/guides/` (R44.15, below). The landing examples use the `sample/` payments corpus only; the `?corpus=ai` swap was dropped when the hero became the Claude Code terminal card (R44.11), generated at build time from `src/components/claude-session.data.json` by `scripts/session-html.ts` (edit the data file only; `check-session-quotes.mjs` verifies quotes verbatim against `sample/` and against the raw capture `scripts/demo-capture.json`, before and after the build), and `sample-ai/` survives as the "AI startup" guide page.
* Two-path strip under the hero (agent card) + `#agent-skill` both carry `npx skills add Atsirkunov/anchorcore --skill anchorcore-setup` — keep the two in sync if the CLI syntax changes (verified against `npx skills add Atsirkunov/anchorcore --list`: the repo exposes `anchorcore-setup` and `aida-landing-page`)
* `src/style.css` `:root` tokens — change `--sage` or `--paper` once
* Header nav exists twice on purpose — `.nav` (desktop, ≥1081px) and the `.navmenu` `<details>` disclosure (≤1080px, works without JS); edit both when adding a section link (R18.1/R18.3)
* Type scale (R44.14): five size tokens on `:root` in `src/style.css` — `--fs-cap` (13px caption: chips, badges, table cells, figure labels), `--fs-body` (`clamp(16px,1vw,18px)`, also nav/buttons/inputs), `--fs-lg` (~21, card/FAQ/guide h3), `--fs-xl` (28+, h2/price), `--fs-display` (40-64, h1; the ≤600px floors are overridden in the phone media block). Use `font-size:var(--fs-*)` only, no literal px sizes; nothing under 12px. Change the token, not the rule. Widescreen growth is inside the clamps (`--wrap`, `--wrap-narrow`, `--pad-sec` likewise, R18.16).
* Text roles: mono (`ui-monospace`) is only for code, commands, file names/identifiers and terminal output (`code`, `.code`, `.term-body`, `.cs-body .q`/`.cs-tool`, TOC/graph file and edge labels). Eyebrows, figure labels, footer, table headers, breadcrumbs and chips are sans. Colour: body text `--ink`, muted `--dim`; `--sageH` is only the primary-button hover. No hex outside the `:root` token block in `style.css`; the inline brand-mark SVG is coloured by `.mk-*` classes. `public/*.svg` are `<img>`s (CSS vars do not apply), so their hexes must equal token values (`flow.svg` grey is `--dim` `#5C6470`).
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
* `npm run guides` (run by `dev` and `build`) writes `guides/<slug>.html` (gitignored intermediates; never commit). Each page lifts the header and footer from `team.html` (Guides link marked current), so nav edits there flow through. Vite builds them to `dist/guides/<slug>.html`, which `html_handling: auto-trailing-slash` serves at `/guides/<slug>` (same mechanism as `/privacy`). `/guides` is an index page generated by the same script from `guides.json` (`guides/index.html`, a Vite entry and a sitemap URL); guide breadcrumbs and the nav "Guides" link on non-landing pages point to it, while the landing nav keeps the `#guides` anchor.
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

## Claude Code session card: real capture

The card shows a real run of AnchorCore's MCP `search` tool on the `sample/` dataset. Claude Code calls `anchorcore - search (MCP)(query: "billing provider migration rollback plan owner", k: 8)`, reads the hits, and writes the answer itself; AnchorCore does not compose it. Only the `search` response is captured data; the answer text in `claude-session.data.json` is authored from facts present in the shown hits.

`scripts/demo-capture.json` is the raw `search` response (never edit it). Captured with AnchorCore 1.0.32 built from commit `6e86b3e` (includes the R43.28 test-env fix; older builds mis-rank this corpus). To regenerate:

1. Run the app on a spare port with a fresh data dir: `ANCHOR_OPEN_BROWSER=0 ANCHOR_SECRETS_NO_KEYRING=1 ANCHOR_TRAY=0 anchorcore --port 8794 --data-dir <tmp> --no-tray`. Copy `sample/` to a neutral path first (hits carry absolute `source_ref` paths).
2. Add it as a folder source labelled `public` (`GET /csrf`, then `POST /sources` and `POST /sources/1/sync` with `x-csrf-token` and `Origin` headers); wait for `GET /sources/jobs` to report `done`.
3. Run `ANCHOR_BACKEND_URL=http://127.0.0.1:8794 anchorcore mcp`, send `initialize`, `notifications/initialized`, then `tools/call` `search` with `{"query": ..., "k": 8}`; save the JSON text of the result.
4. Update `tool`, each source's `rank`/`lines`/`quotes` and the answer in the data file; `npm run build` fails if a quote, path, line range, query or result count disagrees with the capture.
