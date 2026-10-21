// Build-time guard for the simulated Claude Code session (R44.11 stand-in):
// every quoted snippet in src/components/claude-session.data.json must be a
// verbatim substring of its sample/ file (whitespace runs collapsed, since the
// markdown is hard-wrapped), and index.html's static frames must carry the same
// quotes. Exits 1 on any miss so `npm run build` fails.
import { readFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const sample = join(here, "..", "..", "sample");
const data = JSON.parse(readFileSync(join(here, "..", "src", "components", "claude-session.data.json"), "utf8"));
const html = readFileSync(join(here, "..", "index.html"), "utf8");
const norm = (s) => s.replace(/\s+/g, " ");
const esc = (s) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");

let bad = 0;
for (const src of data.sources) {
  let text;
  try {
    text = norm(readFileSync(join(sample, src.file), "utf8"));
  } catch {
    console.error(`check-session-quotes: missing sample file ${src.file}`);
    bad++;
    continue;
  }
  for (const q of src.quotes) {
    if (!text.includes(norm(q))) {
      console.error(`check-session-quotes: quote not verbatim in sample/${src.file}:\n  "${q}"`);
      bad++;
    }
    const n = html.split(`“${esc(q)}”`).length - 1;
    if (n < 2) {
      console.error(`check-session-quotes: index.html static frames must carry the quote twice (hero + #demo), found ${n}:\n  "${q}"`);
      bad++;
    }
  }
}
if (bad) process.exit(1);
console.log(`check-session-quotes: ok (${data.sources.reduce((n, s) => n + s.quotes.length, 0)} quotes)`);
