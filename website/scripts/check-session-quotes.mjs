// Build-time guard for the Claude Code terminal card (R44.11): every quoted
// snippet in src/components/claude-session.data.json must be a verbatim
// substring of its sample/ file (whitespace runs collapsed, since the markdown
// is hard-wrapped). With `--dist` it also checks the BUILT dist/index.html
// (generated from the same data file by scripts/session-html.ts) carries every
// quote twice (hero + #demo). Exits 1 on any miss so `npm run build` fails.
import { readFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const sample = join(here, "..", "..", "sample");
const data = JSON.parse(readFileSync(join(here, "..", "src", "components", "claude-session.data.json"), "utf8"));
const dist = process.argv.includes("--dist");
const html = dist ? readFileSync(join(here, "..", "dist", "index.html"), "utf8") : "";
const norm = (s) => s.replace(/\s+/g, " ");
const esc = (s) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");

let bad = 0;
for (const src of data.sources) {
  let text = "";
  if (!dist) {
    try {
      text = norm(readFileSync(join(sample, src.file), "utf8"));
    } catch {
      console.error(`check-session-quotes: missing sample file ${src.file}`);
      bad++;
      continue;
    }
  }
  for (const q of src.quotes) {
    if (!dist && !text.includes(norm(q))) {
      console.error(`check-session-quotes: quote not verbatim in sample/${src.file}:\n  "${q}"`);
      bad++;
    }
    if (dist) {
      const n = html.split(`“${esc(q)}”`).length - 1;
      if (n < 2) {
        console.error(`check-session-quotes: dist/index.html must carry the quote twice (hero + #demo), found ${n}:\n  "${q}"`);
        bad++;
      }
    }
  }
}
if (bad) process.exit(1);
console.log(`check-session-quotes${dist ? " (dist)" : ""}: ok (${data.sources.reduce((n, s) => n + s.quotes.length, 0)} quotes)`);
