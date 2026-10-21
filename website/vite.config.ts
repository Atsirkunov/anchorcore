import { defineConfig, type Plugin } from "vite";
import { readFileSync } from "node:fs";

// Guides are prerendered by scripts/build-guides.mjs into guides/<slug>.html (R44.15);
// the same list drives the Vite entries and the sitemap, so they cannot drift.
const guides: { slug: string }[] = JSON.parse(readFileSync(new URL("./scripts/guides.json", import.meta.url), "utf8"));
const STATIC_PAGES = ["/", "/privacy", "/team", "/hosted"];
const ORIGIN = "https://anchorcore.dev";

function sitemap(): Plugin {
  return {
    name: "anchorcore-sitemap",
    generateBundle() {
      const urls = [...STATIC_PAGES, ...guides.map((g) => `/guides/${g.slug}`)];
      const body = urls.map((u) => `  <url><loc>${ORIGIN}${u}</loc></url>`).join("\n");
      this.emitFile({
        type: "asset",
        fileName: "sitemap.xml",
        source: `<?xml version="1.0" encoding="UTF-8"?>\n<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n${body}\n</urlset>\n`,
      });
    },
  };
}

export default defineConfig({
  plugins: [sitemap()],
  server: { port: 5174 },
  build: {
    outDir: "dist",
    rollupOptions: {
      // static entries: the landing page, the GDPR Art. 13 notice (R18.11), the
      // audience pages /team + /hosted (R44.16) and the prerendered guides (R44.15);
      // each gets its clean route from its HTML path (html_handling: auto-trailing-slash)
      input: {
        main: "index.html",
        privacy: "privacy.html",
        team: "team.html",
        hosted: "hosted.html",
        ...Object.fromEntries(guides.map((g) => [`guides/${g.slug}`, `guides/${g.slug}.html`])),
      },
    },
  },
});
