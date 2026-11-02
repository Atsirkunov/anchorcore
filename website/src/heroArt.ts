// Hero band: a field of neurons drawn as line art. One search hops keyword → keyword
// (billing → migration → rollback), grey probes hit irrelevant cells and miss, then the
// cited source fires. Plays once (~2.5 s) and rests; reduced motion gets the end state.
// Seeded, so every visitor sees the same drawing. Colours come from theme tokens (style.css .nx-*/.lx-*).

const W = 1600;
const H = 300;
const NS = "http://www.w3.org/2000/svg";
const NODES: [number, number][] = [[80, 180], [370, 115], [680, 178], [950, 105], [1170, 150]];
const WORDS = ["billing", "migration", "rollback"];
const HIT_LABEL = "S2 · lines 6–20"; // the runbook passage the demo below cites
const STEP = 480; // ms per hop

type Pt = [number, number];
type Attrs = Record<string, string | number>;

function seeded(seed: number): () => number {
  let s = seed;
  return () => { s = (s * 16807) % 2147483647; return (s - 1) / 2147483646; };
}

function make(svg: SVGSVGElement) {
  return <K extends keyof SVGElementTagNameMap>(name: K, attrs: Attrs, text?: string, parent: Element = svg): SVGElementTagNameMap[K] => {
    const e = document.createElementNS(NS, name);
    for (const [k, v] of Object.entries(attrs)) e.setAttribute(k, String(v));
    if (text != null) e.textContent = text;
    parent.appendChild(e);
    return e;
  };
}

type El = ReturnType<typeof make>;

/** Dendrite field: clustered cell bodies with branching trees. Returns cell centres. */
function drawTissue(el: El, rnd: () => number): Pt[] {
  const gauss = () => (rnd() + rnd() + rnd() - 1.5) / 1.5;
  let thick = "";
  let thin = "";
  const cells: Pt[] = [];
  const bodies = el("g", {});
  const grow = (x: number, y: number, ang: number, len: number, depth: number): void => {
    if (depth === 0 || len < 3) return;
    const x2 = x + Math.cos(ang) * len, y2 = y + Math.sin(ang) * len;
    const mx = (x + x2) / 2 + (rnd() - .5) * len * .35, my = (y + y2) / 2 + (rnd() - .5) * len * .35;
    const seg = `M${x.toFixed(1)},${y.toFixed(1)}Q${mx.toFixed(1)},${my.toFixed(1)} ${x2.toFixed(1)},${y2.toFixed(1)}`;
    if (depth >= 3) thick += seg; else thin += seg;
    const n = rnd() < .3 ? 3 : 2;
    for (let i = 0; i < n; i++) grow(x2, y2, ang + (rnd() - .5) * 1.4, len * (.6 + rnd() * .15), depth - 1);
  };
  const soma = (x: number, y: number, size: number): void => {
    const k = 3 + Math.floor(rnd() * 3);
    for (let i = 0; i < k; i++) grow(x, y, (i / k) * Math.PI * 2 + rnd() * .7, size * (1.4 + rnd() * .9), 4);
    cells.push([x, y]);
    el("ellipse", { cx: x.toFixed(1), cy: y.toFixed(1), rx: (size * .28).toFixed(1), ry: (size * .22).toFixed(1), class: "lx-body" }, undefined, bodies);
  };
  const clusters: Pt[] = [[180, 120], [420, 210], [640, 90], [880, 200], [1100, 100], [1330, 190], [1520, 80]];
  for (const [cx, cy] of clusters) for (let i = 0; i < 9; i++) soma(cx + gauss() * 110, cy + gauss() * 70, 8 + rnd() * 9);
  for (let i = 0; i < 34; i++) soma(rnd() * W, 15 + rnd() * (H - 30), 7 + rnd() * 8);
  el("path", { d: thin, class: "lx-faint", "stroke-width": ".5" });
  el("path", { d: thick, class: "lx-faint", "stroke-width": ".9" });
  bodies.parentNode?.appendChild(bodies); // cell bodies above the dendrites
  return cells;
}

/** Phones crop to the last hops; labels keep ~12.5px on screen whatever the band's scale. */
function fitView(svg: SVGSVGElement): void {
  const narrow = matchMedia("(max-width: 640px)");
  const fit = (): void => {
    const vb = narrow.matches ? [560, 0, 860, 300] : [0, 0, W, H];
    const box = svg.getBoundingClientRect();
    svg.setAttribute("viewBox", vb.join(" "));
    const k = Math.max(box.width / vb[2], box.height / vb[3]) || 1;
    svg.style.setProperty("--lx-fs", `${Math.max(12, 12.5 / k).toFixed(1)}px`);
  };
  fit();
  narrow.addEventListener("change", fit);
  addEventListener("resize", fit);
}

interface Search {
  segs: SVGPathElement[];
  leads: { i: number; el: SVGPathElement }[];
  misses: { i: number; el: SVGGElement }[];
  dots: SVGCircleElement[];
  labels: SVGTextElement[];
  bang: SVGGElement;
  burst: SVGGElement;
  ring: SVGCircleElement;
  hit: SVGTextElement;
}

function drawRoute(el: El): SVGPathElement[] {
  const segs: SVGPathElement[] = [];
  for (let j = 1; j < NODES.length; j++) {
    const a = NODES[j - 1], b = NODES[j], up = j % 2 ? -1 : 1;
    const d = `M${a} C${a[0] + 110},${a[1] + up * 55} ${b[0] - 110},${b[1] - up * 45} ${b}`;
    el("path", { d, class: "nx-halo" });
    segs.push(el("path", { d, class: "nx-route" }));
  }
  return segs;
}

/** From the start and each keyword, grey probes reach real nearby cells and come up empty. */
function drawMisses(el: El, rnd: () => number, cells: Pt[], s: Pick<Search, "leads" | "misses">): void {
  const nearPath = (q: Pt) => NODES.some((n) => Math.hypot(n[0] - q[0], n[1] - q[1]) < 45);
  NODES.slice(0, 4).forEach((p, i) => {
    const cand = cells.filter((q) => {
      const dd = Math.hypot(q[0] - p[0], q[1] - p[1]);
      return dd > 55 && dd < 210 && q[1] > 25 && q[1] < 235 && !nearPath(q);
    });
    cand.sort(() => rnd() - .5);
    for (const q of cand.slice(0, i === 0 ? 3 : 4)) {
      const mx = (p[0] + q[0]) / 2 + (rnd() - .5) * 50, my = (p[1] + q[1]) / 2 + (rnd() - .5) * 50;
      const lead = el("path", { d: `M${p} Q${mx.toFixed(0)},${my.toFixed(0)} ${q[0].toFixed(0)},${q[1].toFixed(0)}`, class: "nx-lead" });
      const m = el("g", { class: "nx-miss", transform: `translate(${q[0].toFixed(0)},${q[1].toFixed(0)})` });
      el("circle", { r: 6 }, undefined, m);
      el("path", { d: "M-3,-3L3,3M3,-3L-3,3" }, undefined, m);
      s.leads.push({ i, el: lead });
      s.misses.push({ i, el: m });
    }
  });
}

function drawSearch(el: El, rnd: () => number, cells: Pt[]): Search {
  const segs = drawRoute(el);
  const probes: Pick<Search, "leads" | "misses"> = { leads: [], misses: [] };
  drawMisses(el, rnd, cells, probes);
  const dots: SVGCircleElement[] = [];
  const labels: SVGTextElement[] = [];
  NODES.slice(1, 4).forEach((p, i) => {
    dots.push(el("circle", { cx: p[0], cy: p[1], r: 3.5, class: "nx-node" }));
    labels.push(el("text", { x: p[0] + 10, y: p[1] - 10, class: "nx-word" }, WORDS[i]));
  });
  const T = NODES[4];
  const bang = el("g", { class: "nx-bang", transform: `translate(${T})` });
  const burst = el("g", {}, undefined, bang);
  for (let r = 0; r < 14; r++) {
    const t = r / 14 * Math.PI * 2, r0 = 12, r1 = r % 2 ? 22 : 30;
    el("line", { x1: (r0 * Math.cos(t)).toFixed(1), y1: (r0 * Math.sin(t)).toFixed(1), x2: (r1 * Math.cos(t)).toFixed(1), y2: (r1 * Math.sin(t)).toFixed(1) }, undefined, burst);
  }
  const ring = el("circle", { r: 6, class: "nx-ring" }, undefined, burst);
  el("circle", { r: 5, class: "nx-core" }, undefined, burst);
  const hit = el("text", { x: T[0] + 40, y: T[1] + 4, class: "lx-label nx-hit" }, HIT_LABEL);
  el("circle", { cx: NODES[0][0], cy: NODES[0][1], r: 3, class: "lx-dot" });
  return { segs, ...probes, dots, labels, bang, burst, ring, hit };
}

function finalState(s: Search): void {
  for (const p of s.segs) p.style.strokeDashoffset = "0";
  for (const l of s.leads) { l.el.style.strokeDashoffset = "0"; l.el.style.opacity = ".5"; }
  for (const m of s.misses) m.el.style.opacity = ".8";
  for (const e of [...s.dots, ...s.labels]) e.classList.add("on");
  s.bang.style.opacity = "1";
  s.hit.style.opacity = "1";
}

function play(s: Search): void {
  const A = (e: Element, k: Keyframe[], o: KeyframeAnimationOptions) => e.animate(k, { fill: "both", easing: "ease-in-out", ...o });
  const len = (p: SVGPathElement) => p.dataset.len ?? "0";
  s.segs.forEach((p, i) => {
    const t = 300 + i * STEP;
    A(p, [{ strokeDashoffset: len(p) }, { strokeDashoffset: 0 }], { delay: t, duration: STEP * .8 });
    if (i < 3) setTimeout(() => { s.dots[i].classList.add("on"); s.labels[i].classList.add("on"); }, t + STEP * .8);
  });
  const fire = (i: number) => (i === 0 ? 120 : 300 + (i - 1) * STEP + STEP * .8);
  s.leads.forEach((l, k) => A(l.el, [{ strokeDashoffset: len(l.el), opacity: 1 }, { strokeDashoffset: 0, opacity: 1, offset: .5 }, { strokeDashoffset: 0, opacity: .5 }], { delay: fire(l.i) + (k % 3) * 60, duration: 600 }));
  s.misses.forEach((m, k) => A(m.el, [{ opacity: 0 }, { opacity: 1, offset: .35 }, { opacity: .8 }], { delay: fire(m.i) + (k % 3) * 60 + 280, duration: 420 }));
  const tb = 300 + 4 * STEP - STEP * .2;
  A(s.bang, [{ opacity: 0 }, { opacity: 1 }], { delay: tb, duration: 150 });
  A(s.burst, [{ transform: "scale(.3)" }, { transform: "scale(1.2)", offset: .5 }, { transform: "scale(1)" }], { delay: tb, duration: 380 });
  A(s.ring, [{ r: 6, opacity: .9 }, { r: 44, opacity: 0 }], { delay: tb, duration: 700, easing: "ease-out" });
  A(s.hit, [{ opacity: 0 }, { opacity: 1 }], { delay: tb + 200, duration: 300 });
}

export function initHeroArt(id: string): void {
  const svg = document.getElementById(id);
  if (!(svg instanceof SVGSVGElement)) return;
  const rnd = seeded(11);
  const el = make(svg);
  const cells = drawTissue(el, rnd);
  fitView(svg);
  const s = drawSearch(el, rnd, cells);
  for (const p of [...s.segs, ...s.leads.map((l) => l.el)]) {
    const L = String(p.getTotalLength());
    p.style.strokeDasharray = L;
    p.dataset.len = L;
  }
  if (matchMedia("(prefers-reduced-motion: reduce)").matches) finalState(s);
  else play(s);
}
