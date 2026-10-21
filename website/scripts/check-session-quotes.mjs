// Build-time guard for the Claude Code terminal card (R44.11): every quoted
// snippet in src/components/claude-session.data.json must be a verbatim
// substring of its sample/ file AND of the matching hit in demo-capture.json,
// the raw MCP `search` response the card shows (whitespace runs collapsed, since the markdown
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
// The card shows a REAL `search` run: tool line, result count, source rank /
// path / line range and every quote must match the committed raw capture.
if (!dist) {
  const cap = JSON.parse(readFileSync(join(here, "demo-capture.json"), "utf8"));
  if (cap.query !== data.tool.query) { console.error(`check-session-quotes: tool.query differs from capture ("${cap.query}")`); bad++; }
  if (cap.hits.length !== data.tool.results) { console.error(`check-session-quotes: tool.results ${data.tool.results} != capture hits ${cap.hits.length}`); bad++; }
  if (data.tool.k < cap.hits.length) { console.error("check-session-quotes: tool.k smaller than capture hit count"); bad++; }
  for (const src of data.sources) {
    const hit = cap.hits[src.rank - 1];
    if (!hit) { console.error(`check-session-quotes: ${src.id} rank ${src.rank} not in capture`); bad++; continue; }
    if (!hit.source_ref.replace(/ §\d+$/, "").endsWith("/" + src.file)) { console.error(`check-session-quotes: ${src.id} file ${src.file} != capture ${hit.source_ref}`); bad++; }
    if (hit.line_start !== src.lines[0] || hit.line_end !== src.lines[1]) { console.error(`check-session-quotes: ${src.id} lines ${src.lines} != capture ${hit.line_start}-${hit.line_end}`); bad++; }
    for (const q of src.quotes) {
      if (!norm(hit.content).includes(norm(q))) { console.error(`check-session-quotes: quote not verbatim in capture hit #${src.rank}:\n  "${q}"`); bad++; }
    }
  }
}
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
