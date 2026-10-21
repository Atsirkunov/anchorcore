// Pure renderer for the Claude Code terminal card (R44.11). Shared by the
// browser player (ClaudeSession.ts) and the Vite build plugin
// (scripts/session-html.ts) so the static final frame is generated from the
// data file and cannot drift. No DOM access here.
import data from "./claude-session.data.json";

export interface SessionSource { id: string; rank: number; file: string; lines: [number, number]; kind: string; title: string; quotes: string[] }
export interface AnswerSeg { text: string; cite?: string[] }
export interface SessionData {
  prompt: string;
  title: string;
  tag: string;
  tool: { server: string; name: string; query: string; k: number; results: number };
  footer: string;
  sources: SessionSource[];
  answer: AnswerSeg[];
}

export const SESSION: SessionData = data as SessionData;

const esc = (s: string): string => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");

export const resultLine = (d: SessionData): string => `${d.tool.results} results`;

export function transcript(d: SessionData): string {
  const ans = d.answer.map((s) => s.text + (s.cite ? " " + s.cite.map((c) => `[${c}]`).join(" ") : "")).join("");
  return [
    `Claude Code in ${d.title}, on the ${d.tag}.`,
    `User: ${d.prompt}`,
    `Tool call: ${d.tool.server} ${d.tool.name} (MCP), query: "${d.tool.query}", k: ${d.tool.k}`,
    `Result: ${resultLine(d)}`,
    `Answer: ${ans}`,
    ...d.sources.map((s) => `[${s.id}] ${s.file}:${s.lines[0]}-${s.lines[1]}:${s.quotes.map((q) => `"${q}"`).join(" ")}`),
    d.footer,
  ].join("\n");
}

/** Inner markup of the terminal card (window bar, animated body, transcript, footer). */
export function renderSessionHtml(d: SessionData, idPrefix: string): string {
  const srcs = d.sources.map((s) =>
    `<li class="cs-src"><span class="cs-sid">[${s.id}]</span> <span class="cs-path">${esc(s.file)}:${s.lines[0]}-${s.lines[1]}</span>` +
    s.quotes.map((q) => `<p>“${esc(q)}”</p>`).join("") + `</li>`).join("");
  const ans = d.answer.map((s) =>
    `<span class="cs-seg">${esc(s.text)}` +
    (s.cite ? s.cite.map((c) => ` <sup class="cs-mark" data-src="${c}">[${c}]</sup>`).join("") : "") + `</span>`).join("");
  return `<div class="cs-bar"><span class="cs-dots" aria-hidden="true"><i></i><i></i><i></i></span>` +
    `<span class="cs-title">claude · ${esc(d.title)}</span><span class="cs-tag">${esc(d.tag)}</span>` +
    `<button class="cs-replay" type="button" id="${idPrefix}-replay">Replay</button></div>` +
    `<div class="cs-body" aria-hidden="true">` +
    `<div class="cs-head"><span class="cs-star">✻</span> Claude Code</div>` +
    `<div class="cs-prompt"><span class="cs-gt">&gt;</span> <span class="cs-ptext">${esc(d.prompt)}</span></div>` +
    `<div class="cs-tool"><div class="cs-row"><span class="cs-dot">●</span><span><b>${esc(d.tool.server)} - ${esc(d.tool.name)}</b> (MCP)(query: "${esc(d.tool.query)}", k: ${d.tool.k})</span></div>` +
    `<div class="cs-row cs-result"><span class="cs-elbow">⎿</span><span>${esc(resultLine(d))}</span></div></div>` +
    `<div class="cs-row cs-answer"><span class="cs-dot">●</span><span class="cs-atext">${ans}</span></div>` +
    `<div class="cs-sources"><div class="cs-sh">Sources</div><ul>${srcs}</ul></div></div>` +
    `<div class="sr-only">${esc(transcript(d)).replace(/\n/g, "<br>")}</div>` +
    `<div class="cs-foot"><span>${esc(d.footer)}</span></div>`;
}
