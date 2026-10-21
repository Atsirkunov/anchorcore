// Every internal link and #anchor in dist/**/*.html must resolve (no deps, a few ms).
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const dist = join(dirname(fileURLToPath(import.meta.url)), "..", "dist");
const walk = (d) => readdirSync(d).flatMap((f) => (statSync(join(d, f)).isDirectory() ? walk(join(d, f)) : [join(d, f)]));
const rel = (f) => "/" + f.slice(dist.length + 1).replace(/\\/g, "/");
const files = walk(dist);
const pages = files.filter((f) => f.endsWith(".html"));
const urlOf = (f) => rel(f).replace(/\/index\.html$/, "").replace(/\.html$/, "") || "/";
const ids = new Map();
const html = new Map();
for (const f of pages) {
  const s = readFileSync(f, "utf8");
  html.set(urlOf(f), s);
  ids.set(urlOf(f), new Set([...s.matchAll(/\sid="([^"]+)"/g)].map((m) => m[1])));
}
const assets = new Set(files.map(rel));
const bad = [];
for (const [page, s] of html) {
  for (const m of s.matchAll(/<a\s[^>]*?href="([^"]*)"/g)) {
    const href = m[1].replace(/&amp;/g, "&");
    if (/^([a-z][a-z0-9+.-]*:|\/\/)/i.test(href)) continue;
    const [p, hash] = href.split("#");
    const target = p === "" ? page : p.replace(/\/$/, "") || "/";
    if (!html.has(target) && !assets.has(target)) bad.push(`${page}: ${href} (no such page)`);
    else if (hash && html.has(target) && !ids.get(target).has(hash)) bad.push(`${page}: ${href} (no such anchor)`);
  }
}
if (bad.length) {
  console.error("check-links: broken internal links:\n" + bad.join("\n"));
  process.exit(1);
}
console.log(`check-links: ok (${pages.length} pages)`);
