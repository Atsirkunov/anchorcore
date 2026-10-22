// AppSession — the hero's "Browser app" card. Same contract as ClaudeSession
// (R44.11): the final frame is real HTML in index.html, so crawlers and
// no-JS/reduced-motion visitors see a complete answer, and the animation only
// reveals existing elements (visibility / opacity) inside a pinned height.
// The answer is verbatim from scripts/app-capture.json; check-session-quotes
// verifies it and the card's quotes at build time.

const wait = (ms: number): Promise<void> => new Promise((r) => setTimeout(r, ms));

// Returns a play() trigger: the card sits in a hidden tab panel, so the caller
// starts it when the panel is shown instead of an IntersectionObserver.
export function initAppSession(rootId: string): () => void {
  const root = document.getElementById(rootId);
  const noop = (): void => {};
  if (!root) return noop;
  const body = root.querySelector<HTMLElement>(".ap-body");
  const q = root.querySelector<HTMLElement>(".ap-q");
  const btn = root.querySelector<HTMLElement>(".ap-btn");
  const answer = root.querySelector<HTMLElement>(".ap-answer");
  const sources = root.querySelector<HTMLElement>(".ap-sources");
  const replay = root.querySelector<HTMLButtonElement>(".ap-replay");
  if (!body || !q || !btn || !answer || !sources) return noop;
  if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) {
    replay?.setAttribute("hidden", ""); // static final frame only
    return noop;
  }
  const question = q.textContent ?? "";
  const segs = [...answer.querySelectorAll<HTMLElement>(".ap-seg")];
  const finalText = segs.map((s) => s.textContent ?? "");
  const marks = [...answer.querySelectorAll<HTMLElement>(".ap-mark")];
  const cards = [...sources.querySelectorAll<HTMLElement>(".ap-src")];
  const caret = document.createElement("span");
  caret.className = "ap-caret";
  let run = 0;

  const reset = (): void => {
    body.classList.add("ap-playing");
    q.textContent = "";
    btn.textContent = "Ask";
    btn.classList.remove("busy");
    answer.classList.remove("on");
    sources.classList.remove("on");
    segs.forEach((s, i) => { s.classList.remove("on"); s.textContent = finalText[i]; });
    marks.forEach((m) => m.classList.remove("on"));
    cards.forEach((c) => c.classList.remove("on", "hit"));
    caret.remove();
  };

  const flash = (id: string | undefined): void => {
    const card = cards.find((c) => c.dataset.id === id);
    if (!card) return;
    card.classList.add("hit");
    setTimeout(() => card.classList.remove("hit"), 900);
  };

  async function play(): Promise<void> {
    if (!body || !q || !btn || !answer || !sources) return; // narrowing does not survive into function declarations
    const my = ++run;
    const live = (): boolean => my === run;
    // Pin the final-frame height so streaming text can never resize the card (CLS 0)
    if (!body.classList.contains("ap-playing")) body.style.height = `${body.offsetHeight}px`;
    reset();
    q.after(caret);
    for (let i = 1; i <= question.length; i++) {
      q.textContent = question.slice(0, i);
      await wait(22);
      if (!live()) return;
    }
    await wait(300);
    if (!live()) return;
    caret.remove();
    btn.textContent = "Thinking…";
    btn.classList.add("busy");
    sources.classList.add("on"); // retrieval lands before the answer streams
    for (const card of cards) {
      card.classList.add("on");
      await wait(160);
      if (!live()) return;
    }
    await wait(500);
    if (!live()) return;
    answer.classList.add("on");
    for (const seg of segs) {
      const n = segs.indexOf(seg);
      seg.textContent = "";
      seg.classList.add("on");
      seg.after(caret);
      for (const w of finalText[n].split(/(?<= )/)) {
        seg.textContent += w;
        await wait(38);
        if (!live()) return;
      }
      const mark = seg.nextElementSibling;
      if (mark instanceof HTMLElement && mark.classList.contains("ap-mark")) {
        mark.after(caret);
        mark.classList.add("on");
        flash(mark.dataset.cite);
        await wait(320);
        if (!live()) return;
      }
    }
    caret.remove();
    btn.textContent = "Ask";
    btn.classList.remove("busy");
    await wait(600);
    if (!live()) return;
    body.classList.remove("ap-playing");
    body.style.height = "";
  }

  replay?.addEventListener("click", () => void play());
  // Hovering a marker points at its source card, animated or not
  for (const m of marks) m.addEventListener("mouseenter", () => flash(m.dataset.cite));
  let started = false;
  return () => {
    if (started) return;
    started = true;
    void play();
  };
}
