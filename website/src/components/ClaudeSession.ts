// ClaudeSession (R44.11 stand-in) — scripted SIMULATION of a Claude Code
// session calling AnchorCore, built from the sample/ corpus. Not a recording.
// The final frame is real HTML (index.html carries the same markup, produced
// by renderSessionHtml), so crawlers and no-JS/reduced-motion visitors see a
// complete answer. Animation only reveals existing elements (opacity /
// visibility), so layout never shifts. Quotes are verified verbatim against
// sample/ by scripts/check-session-quotes.mjs at build time.
import data from "./claude-session.data.json";

export interface SessionSource { id: string; file: string; kind: string; title: string; tone: "sage" | "stone"; quotes: string[] }
export interface AnswerSeg { text: string; cite?: string[] }
export interface SessionData {
  prompt: string;
  tool: { name: string; done: string };
  simLabel: string;
  footer: string;
  sources: SessionSource[];
  answer: AnswerSeg[];
}

export const SESSION: SessionData = data as SessionData;

const esc = (s: string): string => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");

export function transcript(d: SessionData): string {
  const ans = d.answer.map((s) => s.text + (s.cite ? " " + s.cite.map((c) => `[${c}]`).join(" ") : "")).join("");
  return [
    `Simulation, not a recording: ${d.simLabel}.`,
    `User: ${d.prompt}`,
    `Tool call: ${d.tool.name}`,
    ...d.sources.map((s) => `[${s.id}] ${s.file}: ${s.quotes.map((q) => `"${q}"`).join(" ")}`),
    `Answer: ${ans}`,
    d.footer,
  ].join("\n");
}

/** Inner markup of the session card (bar, animated body, transcript, footer). */
export function renderSessionHtml(d: SessionData, idPrefix: string): string {
  const cards = d.sources.map((s) =>
    `<div class="cite cs-card" data-src="${s.id}"><span class="bar ${s.tone}"></span><div><b>${s.id} · ${esc(s.title)}</b> <small>${esc(s.kind)}</small>` +
    s.quotes.map((q) => `<p>“${esc(q)}”</p>`).join("") +
    `<small>${esc(s.file)}</small></div></div>`).join("");
  const ans = d.answer.map((s) =>
    `<span class="cs-seg">${esc(s.text)}` +
    (s.cite ? s.cite.map((c) => ` <sup class="cs-mark" data-src="${c}">[${c}]</sup>`).join("") : "") + `</span>`).join("");
  return `<div class="shot-bar cs-bar"><span>Claude Code · <em>${esc(d.simLabel)}</em> (simulation, not a recording)</span>` +
    `<button class="btn small secondary cs-replay" type="button" id="${idPrefix}-replay">Replay</button></div>` +
    `<div class="cs-body" aria-hidden="true">` +
    `<div class="cs-prompt q"><span class="cs-gt">&gt;</span> <span class="cs-ptext">${esc(d.prompt)}</span></div>` +
    `<div class="cs-tool"><span class="cs-tname">${esc(d.tool.name)}</span> <span class="cs-tstate">${esc(d.tool.done)}</span></div>` +
    `<div class="cites cs-cards">${cards}</div>` +
    `<div class="a cs-answer">${ans}</div></div>` +
    `<div class="sr-only">${esc(transcript(d)).replace(/\n/g, "<br>")}</div>` +
    `<div class="shot-foot cs-foot">${esc(d.footer)}</div>`;
}

const wait = (ms: number): Promise<void> => new Promise((r) => setTimeout(r, ms));

export function initClaudeSession(rootId: string): void {
  const root = document.getElementById(rootId);
  if (!root) return;
  const body = root.querySelector<HTMLElement>(".cs-body");
  const replay = root.querySelector<HTMLButtonElement>(".cs-replay");
  const ptext = root.querySelector<HTMLElement>(".cs-ptext");
  const tool = root.querySelector<HTMLElement>(".cs-tool");
  const answer = root.querySelector<HTMLElement>(".cs-answer");
  if (!body || !ptext || !tool || !answer) return;
  if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) {
    replay?.setAttribute("hidden", ""); // static final frame only
    return;
  }
  const cards = [...body.querySelectorAll<HTMLElement>(".cs-card")];
  const segs = [...body.querySelectorAll<HTMLElement>(".cs-seg")];
  const finalText = segs.map((s) => s.firstChild?.textContent ?? "");
  const prompt = SESSION.prompt;
  let run = 0;

  const reset = (): void => {
    body.classList.add("cs-playing");
    ptext.textContent = "";
    tool.classList.remove("on", "done");
    cards.forEach((c) => c.classList.remove("on", "lit"));
    segs.forEach((s) => {
      s.classList.remove("on");
      if (s.firstChild) s.firstChild.textContent = finalText[segs.indexOf(s)];
      s.querySelectorAll(".cs-mark").forEach((m) => m.classList.remove("on"));
    });
    answer.classList.remove("on");
  };

  async function play(): Promise<void> {
    if (!body || !ptext || !tool || !answer) return; // narrowing does not survive into function declarations
    const my = ++run;
    const live = (): boolean => my === run;
    // Pin the final-frame height so streaming text can never resize the card (CLS 0)
    if (!body.classList.contains("cs-playing")) body.style.height = `${body.offsetHeight}px`;
    reset();
    for (let i = 1; i <= prompt.length; i++) {
      ptext.textContent = prompt.slice(0, i);
      await wait(20);
      if (!live()) return;
    }
    await wait(350);
    if (!live()) return;
    tool.classList.add("on");
    await wait(1100);
    if (!live()) return;
    tool.classList.add("done");
    for (const c of cards) {
      c.classList.add("on");
      await wait(550);
      if (!live()) return;
    }
    answer.classList.add("on");
    for (let n = 0; n < segs.length; n++) {
      const seg = segs[n];
      const node = seg.firstChild;
      if (node) node.textContent = "";
      seg.classList.add("on");
      for (const w of finalText[n].split(/(?<= )/)) {
        if (node) node.textContent += w;
        await wait(32);
        if (!live()) return;
      }
      for (const m of seg.querySelectorAll<HTMLElement>(".cs-mark")) {
        m.classList.add("on");
        const card = cards.find((c) => c.dataset.src === m.dataset.src);
        card?.classList.add("lit");
        setTimeout(() => card?.classList.remove("lit"), 1100);
        await wait(250);
        if (!live()) return;
      }
    }
    await wait(1200);
    if (!live()) return;
    body.classList.remove("cs-playing");
    body.style.height = "";
  }

  replay?.addEventListener("click", () => void play());
  if ("IntersectionObserver" in window) {
    const obs = new IntersectionObserver((entries) => {
      if (entries.some((e) => e.isIntersecting)) {
        void play();
        obs.disconnect();
      }
    }, { threshold: 0.4 });
    obs.observe(root);
  }
}
