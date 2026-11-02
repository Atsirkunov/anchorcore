// Hero band drawing, generated once at build (scripts/hero-art.ts injects it into index.html).
// Pure string output, no DOM: a seeded field of neurons with one search route
// (billing → migration → rollback → the cited source) and grey probes that miss.
// src/heroArt.ts animates it in the browser; colours come from style.css (.lx-*/.nx-*).

export const VIEW_W = 1600;
export const VIEW_H = 300;
const NODES: [number, number][] = [[80, 180], [370, 115], [680, 178], [950, 105], [1170, 150]];
const WORDS = ["billing", "migration", "rollback"];
const MIN_TWIG = 3; // px; shorter branches are not drawn
const HIT_LABEL = "S2 · lines 6–20"; // the runbook passage the demo below cites

type Pt = [number, number];
type Attrs = Record<string, string | number>;

function seeded(seed: number): () => number {
  let s = seed;
  return () => { s = (s * 16807) % 2147483647; return (s - 1) / 2147483646; };
}

const esc = (v: string | number) => String(v).replace(/&/g, "&amp;").replace(/"/g, "&quot;").replace(/</g, "&lt;");
const n1 = (v: number) => v.toFixed(1).replace(/\.0$/, "");

function tag(name: string, attrs: Attrs, inner = ""): string {
  const a = Object.entries(attrs).map(([k, v]) => ` ${k}="${esc(v)}"`).join("");
  return inner === "" && name !== "text" ? `<${name}${a}/>` : `<${name}${a}>${inner}</${name}>`;
}

/** Dendrite field: clustered cell bodies with branching trees. */
function tissue(rnd: () => number): { svg: string; cells: Pt[] } {
  const gauss = () => (rnd() + rnd() + rnd() - 1.5) / 1.5;
  let thick = "";
  let thin = "";
  let bodies = "";
  const cells: Pt[] = [];
  // Compact path data: whole pixels, relative curves, and each branch's first child continues
  // the current subpath (no new M). Twigs under MIN_TWIG px are skipped (invisible at band scale).
  const r = Math.round;
  const grow = (x: number, y: number, ang: number, len: number, depth: number, cont: boolean): void => {
    if (depth === 0 || len < MIN_TWIG) return;
    const x2 = x + Math.cos(ang) * len, y2 = y + Math.sin(ang) * len;
    const mx = (x + x2) / 2 + (rnd() - .5) * len * .35, my = (y + y2) / 2 + (rnd() - .5) * len * .35;
    const ox = r(x), oy = r(y);
    const seg = `${cont ? "" : `M${ox} ${oy}`}q${r(mx) - ox} ${r(my) - oy} ${r(x2) - ox} ${r(y2) - oy}`;
    if (depth >= 3) thick += seg; else thin += seg;
    const n = rnd() < .3 ? 3 : 2;
    // the continued child must start where this segment ended (rounded end point)
    for (let i = 0; i < n; i++) grow(i === 0 ? r(x2) : x2, i === 0 ? r(y2) : y2, ang + (rnd() - .5) * 1.4, len * (.6 + rnd() * .15), depth - 1, i === 0 && depth - 1 >= 3 === depth >= 3);
  };
  const soma = (x: number, y: number, size: number): void => {
    const k = 3 + Math.floor(rnd() * 3);
    for (let i = 0; i < k; i++) grow(x, y, (i / k) * Math.PI * 2 + rnd() * .7, size * (1.4 + rnd() * .9), 4, false);
    cells.push([x, y]);
    bodies += tag("ellipse", { cx: n1(x), cy: n1(y), rx: n1(size * .28), ry: n1(size * .22) });
  };
  const clusters: Pt[] = [[180, 120], [420, 210], [640, 90], [880, 200], [1100, 100], [1330, 190], [1520, 80]];
  for (const [cx, cy] of clusters) for (let i = 0; i < 9; i++) soma(cx + gauss() * 110, cy + gauss() * 70, 8 + rnd() * 9);
  for (let i = 0; i < 34; i++) soma(rnd() * VIEW_W, 15 + rnd() * (VIEW_H - 30), 7 + rnd() * 8);
  const svg = tag("path", { d: thin, class: "lx-faint", "stroke-width": ".5" })
    + tag("path", { d: thick, class: "lx-faint", "stroke-width": ".9" })
    + tag("g", { class: "lx-body" }, bodies);
  return { svg, cells };
}

function route(): string {
  let out = "";
  for (let j = 1; j < NODES.length; j++) {
    const a = NODES[j - 1], b = NODES[j], up = j % 2 ? -1 : 1;
    const d = `M${a} C${a[0] + 110},${a[1] + up * 55} ${b[0] - 110},${b[1] - up * 45} ${b}`;
    out += tag("path", { d, class: "nx-halo" }) + tag("path", { d, class: "nx-route", pathLength: 1 });
  }
  return out;
}

/** From the start and each keyword, grey probes reach real nearby cells and come up empty. */
function misses(rnd: () => number, cells: Pt[]): string {
  const nearPath = (q: Pt) => NODES.some((n) => Math.hypot(n[0] - q[0], n[1] - q[1]) < 45);
  let out = "";
  NODES.slice(0, 4).forEach((p, i) => {
    const cand = cells.filter((q) => {
      const dd = Math.hypot(q[0] - p[0], q[1] - p[1]);
      return dd > 55 && dd < 210 && q[1] > 25 && q[1] < 235 && !nearPath(q);
    });
    cand.sort(() => rnd() - .5);
    for (const q of cand.slice(0, i === 0 ? 3 : 4)) {
      const mx = (p[0] + q[0]) / 2 + (rnd() - .5) * 50, my = (p[1] + q[1]) / 2 + (rnd() - .5) * 50;
      out += tag("path", { d: `M${p} Q${mx.toFixed(0)},${my.toFixed(0)} ${q[0].toFixed(0)},${q[1].toFixed(0)}`, class: "nx-lead", pathLength: 1, "data-hop": i });
      out += tag("g", { class: "nx-miss", "data-hop": i, transform: `translate(${q[0].toFixed(0)},${q[1].toFixed(0)})` },
        tag("circle", { r: 6 }) + tag("path", { d: "M-3,-3L3,3M3,-3L-3,3" }));
    }
  });
  return out;
}

function hit(): string {
  let rays = "";
  for (let r = 0; r < 14; r++) {
    const t = r / 14 * Math.PI * 2, r0 = 12, r1 = r % 2 ? 22 : 30;
    rays += tag("line", { x1: n1(r0 * Math.cos(t)), y1: n1(r0 * Math.sin(t)), x2: n1(r1 * Math.cos(t)), y2: n1(r1 * Math.sin(t)) });
  }
  const T = NODES[4];
  const burst = tag("g", { class: "nx-burst" }, rays + tag("circle", { r: 6, class: "nx-ring" }) + tag("circle", { r: 5, class: "nx-core" }));
  return tag("g", { class: "nx-bang", transform: `translate(${T})` }, burst)
    + tag("text", { x: T[0] + 40, y: T[1] + 4, class: "lx-label nx-hit" }, esc(HIT_LABEL));
}

/** The band's inner SVG markup (deterministic: same output on every build). */
export function neuronSvg(): string {
  const rnd = seeded(11);
  const { svg, cells } = tissue(rnd);
  const keywords = NODES.slice(1, 4).map((p, i) =>
    tag("circle", { cx: p[0], cy: p[1], r: 3.5, class: "nx-node" })
    + tag("text", { x: p[0] + 10, y: p[1] - 10, class: "nx-word" }, WORDS[i])).join("");
  return svg + route() + misses(rnd, cells) + keywords + hit()
    + tag("circle", { cx: NODES[0][0], cy: NODES[0][1], r: 3, class: "lx-dot" });
}
