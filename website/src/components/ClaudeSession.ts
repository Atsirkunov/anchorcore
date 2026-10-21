// ClaudeSession (R44.11) — scripted Claude Code terminal session built from the
// sample/ corpus. The final frame is real HTML (generated at build time from
// claude-session.data.json by scripts/session-html.ts), so crawlers and
// no-JS/reduced-motion visitors see a complete answer. Animation only reveals
// existing elements (visibility / opacity), so layout never shifts. Quotes are
// verified verbatim against sample/ by scripts/check-session-quotes.mjs.
import { SESSION } from "./session-render";

const wait = (ms: number): Promise<void> => new Promise((r) => setTimeout(r, ms));

export function initClaudeSession(rootId: string): void {
  const root = document.getElementById(rootId);
  if (!root) return;
  const body = root.querySelector<HTMLElement>(".cs-body");
  const replay = root.querySelector<HTMLButtonElement>(".cs-replay");
  const ptext = root.querySelector<HTMLElement>(".cs-ptext");
  const tool = root.querySelector<HTMLElement>(".cs-tool");
  const answer = root.querySelector<HTMLElement>(".cs-answer");
  const sources = root.querySelector<HTMLElement>(".cs-sources");
  if (!body || !ptext || !tool || !answer || !sources) return;
  if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) {
    replay?.setAttribute("hidden", ""); // static final frame only
    return;
  }
  const segs = [...body.querySelectorAll<HTMLElement>(".cs-seg")];
  const finalText = segs.map((s) => s.firstChild?.textContent ?? "");
  const prompt = SESSION.prompt;
  const caret = document.createElement("span");
  caret.className = "cs-caret";
  let run = 0;

  const reset = (): void => {
    body.classList.add("cs-playing");
    ptext.textContent = "";
    tool.classList.remove("on", "done");
    sources.classList.remove("on");
    segs.forEach((s, i) => {
      s.classList.remove("on");
      if (s.firstChild) s.firstChild.textContent = finalText[i];
      s.querySelectorAll(".cs-mark").forEach((m) => m.classList.remove("on"));
    });
    answer.classList.remove("on");
    caret.remove();
  };

  async function play(): Promise<void> {
    if (!body || !ptext || !tool || !answer || !sources) return; // narrowing does not survive into function declarations
    const my = ++run;
    const live = (): boolean => my === run;
    // Pin the final-frame height so streaming text can never resize the card (CLS 0)
    if (!body.classList.contains("cs-playing")) body.style.height = `${body.offsetHeight}px`;
    reset();
    ptext.after(caret);
    for (let i = 1; i <= prompt.length; i++) {
      ptext.textContent = prompt.slice(0, i);
      await wait(20);
      if (!live()) return;
    }
    await wait(350);
    if (!live()) return;
    caret.remove();
    tool.classList.add("on");
    await wait(1100);
    if (!live()) return;
    tool.classList.add("done");
    await wait(450);
    if (!live()) return;
    answer.classList.add("on");
    for (let n = 0; n < segs.length; n++) {
      const seg = segs[n];
      const node = seg.firstChild;
      if (node) node.textContent = "";
      seg.classList.add("on");
      if (node) node.after(caret);
      for (const w of finalText[n].split(/(?<= )/)) {
        if (node) node.textContent += w;
        await wait(32);
        if (!live()) return;
      }
      for (const m of seg.querySelectorAll<HTMLElement>(".cs-mark")) {
        m.classList.add("on");
        await wait(250);
        if (!live()) return;
      }
    }
    caret.remove();
    sources.classList.add("on");
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
