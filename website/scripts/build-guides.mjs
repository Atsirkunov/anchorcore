// R44.15: prerender the guides as static pages. Repo markdown (docs/) stays the
// source of truth; this writes website/guides/<slug>.html (gitignored build
// intermediates, the Vite entries for /guides/<slug>). `marked` is build-time only.
// The guide list lives in scripts/guides.json and also feeds vite.config.ts
// (entries + sitemap) so the three can never drift.
import { readFileSync, writeFileSync, mkdirSync, rmSync, existsSync, statSync } from "node:fs";
import { join, dirname, posix } from "node:path";
import { fileURLToPath } from "node:url";
import { Marked } from "marked";

const here = dirname(fileURLToPath(import.meta.url));
const site = join(here, "..");
const repo = join(site, "..");
const OUT = join(site, "guides");
const ORIGIN = "https://anchorcore.dev";
const GH = "https://github.com/Atsirkunov/anchorcore";
const guides = JSON.parse(readFileSync(join(here, "guides.json"), "utf8"));
const bySource = new Map(guides.map((g) => [g.source, g]));

const esc = (s) => String(s).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
const slugify = (s) => s.toLowerCase().replace(/<[^>]*>/g, "").replace(/[^\p{L}\p{N}\s-]/gu, "").trim().replace(/\s+/g, "-");

// Relative links between docs -> the matching site guide, else the GitHub blob/tree URL.
function rewriteHref(href, sourcePath) {
  if (/^(https?:|mailto:)/i.test(href)) return href.replace(/^https:\/\/anchorcore\.dev(?=\/)/, "");
  if (href.startsWith("#") || href.startsWith("/")) return href;
  if (/^[a-z][a-z0-9+.-]*:/i.test(href)) return null; // javascript:, data:, ...
  const [pathPart, hash = ""] = href.split("#");
  const resolved = posix.normalize(posix.join(posix.dirname(sourcePath), pathPart));
  if (resolved.startsWith("..")) return null;
  const guide = bySource.get(resolved);
  if (guide) return `/guides/${guide.slug}${hash ? `#${hash}` : ""}`;
  const full = join(repo, resolved);
  const kind = existsSync(full) && statSync(full).isDirectory() ? "tree" : "blob";
  return `${GH}/${kind}/main/${resolved}${hash ? `#${hash}` : ""}`;
}

function makeMarked(sourcePath) {
  const seen = new Map();
  return new Marked({
    gfm: true,
    renderer: {
      // never pass raw HTML through: show it as text
      html: (token) => esc(token.text),
      heading(token) {
        let id = slugify(token.text);
        const n = seen.get(id) ?? 0;
        seen.set(id, n + 1);
        if (n) id = `${id}-${n}`;
        const inner = this.parser.parseInline(token.tokens);
        return `<h${token.depth} id="${id}">${inner}<a class="h-anchor" href="#${id}" aria-label="Link to this section">#</a></h${token.depth}>\n`;
      },
      link(token) {
        const href = rewriteHref(token.href, sourcePath);
        const inner = this.parser.parseInline(token.tokens);
        if (href === null) return inner;
        const ext = /^https?:/i.test(href);
        const title = token.title ? ` title="${esc(token.title)}"` : "";
        return `<a href="${esc(href)}"${title}${ext ? ' rel="noopener noreferrer"' : ""}>${inner}</a>`;
      },
      image(token) {
        const src = /^https?:/i.test(token.href) ? token.href : `https://raw.githubusercontent.com/Atsirkunov/anchorcore/main/${posix.join(posix.dirname(sourcePath), token.href)}`;
        return `<img src="${esc(src)}" alt="${esc(token.text)}" loading="lazy">`;
      },
      table(token) {
        const head = token.header.map((c) => `<th scope="col">${this.parser.parseInline(c.tokens)}</th>`).join("");
        const rows = token.rows.map((r) => `<tr>${r.map((c) => `<td>${this.parser.parseInline(c.tokens)}</td>`).join("")}</tr>`).join("");
        return `<div class="table-scroll"><table><thead><tr>${head}</tr></thead><tbody>${rows}</tbody></table></div>\n`;
      },
    },
  });
}

// Header/footer are lifted from team.html (R44.16) so nav changes there flow here.
function chrome() {
  const team = readFileSync(join(site, "team.html"), "utf8");
  const header = team.match(/<header class="site-header">[\s\S]*?<\/header>/)?.[0];
  const footer = team.match(/<footer>[\s\S]*?<\/footer>/)?.[0];
  if (!header || !footer) throw new Error("build-guides: team.html header/footer markers not found");
  const cur = ' aria-current="page"';
  return {
    header: header
      .replaceAll(`<a href="/team"${cur}>`, '<a href="/team">')
      .replaceAll('<a href="/#guides">', `<a href="/#guides"${cur}>`),
    footer,
  };
}

function page(g, html, h1, others, chromeParts) {
  const url = `${ORIGIN}/guides/${g.slug}`;
  const title = `${g.title} — AnchorCore`;
  const more = others.map((o) => `<li><a href="/guides/${o.slug}">${esc(o.title)}</a></li>`).join("");
  return `<!doctype html>
<html lang="en">
<head>
  <meta charset="UTF-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>${esc(title)}</title>
  <meta name="description" content="${esc(g.description)}" />
  <link rel="icon" href="/favicon.svg" type="image/svg+xml" />
  <link rel="canonical" href="${url}" />
  <meta property="og:type" content="article" />
  <meta property="og:title" content="${esc(title)}" />
  <meta property="og:description" content="${esc(g.description)}" />
  <meta property="og:url" content="${url}" />
  <meta property="og:image" content="${ORIGIN}/og-card.png" />
  <meta name="twitter:card" content="summary_large_image" />
  <meta name="twitter:image" content="${ORIGIN}/og-card.png" />
  <link rel="stylesheet" href="/src/style.css" />
</head>
<body>
  <!-- GENERATED by scripts/build-guides.mjs from ${g.source}: do not edit, do not commit -->
  ${chromeParts.header}

  <main>
    <article class="guide-page">
      <div class="wrap">
        <p class="crumbs"><a href="/#guides">Guides</a> <span aria-hidden="true">/</span> ${esc(g.title)}</p>
        <h1>${h1}</h1>
        <div class="guide-body">
${html}
        </div>
        <p class="guide-source">Source: <a href="${GH}/blob/main/${g.source}" rel="noopener noreferrer">${g.source}</a> in the public repo.</p>
        <aside class="guide-more" aria-labelledby="more-guides">
          <h2 id="more-guides">More guides</h2>
          <ul>${more}</ul>
          <p><a class="btn primary" href="/#download">Download AnchorCore</a></p>
        </aside>
      </div>
    </article>
  </main>

  ${chromeParts.footer}

  <script type="module" src="/src/guide.ts"></script>
</body>
</html>
`;
}

rmSync(OUT, { recursive: true, force: true });
mkdirSync(OUT, { recursive: true });
const parts = chrome();
for (const g of guides) {
  const md = readFileSync(join(repo, g.source), "utf8");
  const m = md.match(/^# (.*)\n/);
  if (!m) throw new Error(`build-guides: ${g.source} has no H1`);
  const marked = makeMarked(g.source);
  const h1 = marked.parseInline(m[1]);
  const body = marked.parse(md.replace(/^# .*\n/, ""));
  const others = guides.filter((o) => o.slug !== g.slug);
  writeFileSync(join(OUT, `${g.slug}.html`), page(g, body, h1, others, parts));
}
console.log(`build-guides: ${guides.length} pages -> guides/`);
