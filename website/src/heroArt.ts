// Hero band runtime. The drawing itself is static markup generated at build (src/heroArtDraw.ts
// via scripts/hero-art.ts); this only fits it to the viewport and plays the search once (~2.5 s).
// Before this runs, `html.js` CSS hides the route, probes and hit so the first paint is the
// empty field; without JS the finished picture shows. Reduced motion gets the end state.

import { VIEW_H, VIEW_W } from "./heroArtDraw";

const STEP = 480; // ms per hop

/** Phones crop to the last hops; labels keep ~12.5px on screen whatever the band's scale. */
function fitView(svg: SVGSVGElement): void {
  const narrow = matchMedia("(max-width: 640px)");
  const fit = (): void => {
    const vb = narrow.matches ? [560, 0, 860, 300] : [0, 0, VIEW_W, VIEW_H];
    const box = svg.getBoundingClientRect();
    svg.setAttribute("viewBox", vb.join(" "));
    const k = Math.max(box.width / vb[2], box.height / vb[3]) || 1;
    svg.style.setProperty("--lx-fs", `${Math.max(12, 12.5 / k).toFixed(1)}px`);
  };
  fit();
  narrow.addEventListener("change", fit);
  addEventListener("resize", fit);
}

function play(svg: SVGSVGElement, reduced: boolean): void {
  const all = <T extends Element>(sel: string) => [...svg.querySelectorAll<T>(sel)];
  const hop = (e: Element) => Number(e.getAttribute("data-hop") ?? 0);
  const segs = all<SVGPathElement>(".nx-route");
  const lit = [...all<SVGCircleElement>(".nx-node"), ...all<SVGTextElement>(".nx-word")];
  const A = (e: Element | null, k: Keyframe[], o: KeyframeAnimationOptions) =>
    e?.animate(k, { fill: "both", easing: "ease-in-out", ...o, ...(reduced ? { delay: 0, duration: 0 } : {}) });
  segs.forEach((p, i) => {
    const t = 300 + i * STEP;
    A(p, [{ strokeDashoffset: 1 }, { strokeDashoffset: 0 }], { delay: t, duration: STEP * .8 });
    const nodes = lit.filter((_, j) => j % 3 === i); // node i and word i light when the route reaches them
    if (i < 3) setTimeout(() => nodes.forEach((e) => e.classList.add("on")), reduced ? 0 : t + STEP * .8);
  });
  const fire = (i: number) => (i === 0 ? 120 : 300 + (i - 1) * STEP + STEP * .8);
  all(".nx-lead").forEach((l, k) => A(l, [{ strokeDashoffset: 1, opacity: 1 }, { strokeDashoffset: 0, opacity: 1, offset: .5 }, { strokeDashoffset: 0, opacity: .5 }], { delay: fire(hop(l)) + (k % 3) * 60, duration: 600 }));
  all(".nx-miss").forEach((m, k) => A(m, [{ opacity: 0 }, { opacity: 1, offset: .35 }, { opacity: .8 }], { delay: fire(hop(m)) + (k % 3) * 60 + 280, duration: 420 }));
  const tb = 300 + 4 * STEP - STEP * .2;
  A(svg.querySelector(".nx-bang"), [{ opacity: 0 }, { opacity: 1 }], { delay: tb, duration: 150 });
  A(svg.querySelector(".nx-burst"), [{ transform: "scale(.3)" }, { transform: "scale(1.2)", offset: .5 }, { transform: "scale(1)" }], { delay: tb, duration: 380 });
  if (!reduced) A(svg.querySelector(".nx-ring"), [{ r: 6, opacity: .9 }, { r: 44, opacity: 0 }], { delay: tb, duration: 700, easing: "ease-out" });
  A(svg.querySelector(".nx-hit"), [{ opacity: 0 }, { opacity: 1 }], { delay: tb + 200, duration: 300 });
}

export function initHeroArt(id: string): void {
  const svg = document.getElementById(id);
  if (!(svg instanceof SVGSVGElement)) return;
  fitView(svg);
  play(svg, matchMedia("(prefers-reduced-motion: reduce)").matches);
}
