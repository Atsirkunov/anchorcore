import { renderGraphMini } from "./components/GraphMini";
import { marked } from "marked";
import obsidianMd from "../../docs/guides/obsidian-vault.md?raw";
import quickstartMd from "../../docs/sample-dataset.md?raw";
import restMd from "../../docs/guides/rest-api.md?raw";
import aiMd from "../../docs/sample-ai.md?raw";

// tweakable copy — single source, no CMS
const copy = {
  personal: {
    title: "Your memory — finally searchable.",
    sub: "Your notes, PDFs, Jira, Linear and Drive in one searchable private memory. Ask like you remember it — get the source.",
    proof: "300-page PDF? Page 250 still cited. Your second brain, not another chatbot.",
    roadmap: "personal"
  },
  team: {
    title: "Run on your infrastructure — one container.",
    sub: "Self-hosted private AI knowledge base in one command. Your data stays with you, same cited answers for the whole team. One-time platform license, yours to run — no cloud required.",
    proof: "One Docker container — your team's memory, on your infra. Who owns billing? Cited, not guessed.",
    roadmap: "team"
  },
  hosted: {
    title: "Hosted — coming soon.",
    sub: "We host it for you, same private memory, no setup. Join waitlist for early access.",
    proof: "No Docker, no infra — just connect and ask. Your memory, hosted.",
    roadmap: "hosted"
  }
} as const;

type Track = keyof typeof copy;

// Direct download assets (release.yml — keep these file names in sync).
const WIN_ZIP = "https://github.com/Atsirkunov/anchorcore/releases/latest/download/AnchorCore-windows.zip";
const MAC_ZIP = "https://github.com/Atsirkunov/anchorcore/releases/latest/download/AnchorCore-macos.zip";

const LINUX_ZIP = "https://github.com/Atsirkunov/anchorcore/releases/latest/download/AnchorCore-linux.zip";

type Os = "mac" | "windows" | "linux" | "mobile";

// Pure so the UA matrix is testable: phones/tablets first (iPadOS reports a Mac
// platform, so touch points disambiguate), then desktop families.
function detectOs(ua: string, platform: string, touchPoints: number): Os {
  if (/android|iphone|ipad|ipod|mobile/i.test(ua)) return "mobile";
  if (/mac/i.test(platform)) return touchPoints > 1 ? "mobile" : "mac";
  if (/win/i.test(platform) || /windows/i.test(ua)) return "windows";
  if (/linux|x11|cros/i.test(platform + " " + ua)) return "linux";
  return "windows";
}

function currentOs(): Os {
  // userAgentData ships ahead of the DOM lib's Navigator type; keep the fallback
  const ua = (navigator as Navigator & { userAgentData?: { platform?: string } }).userAgentData;
  return detectOs(navigator.userAgent ?? "", ua?.platform ?? navigator.platform ?? "", navigator.maxTouchPoints ?? 0);
}

function isMacOs(): boolean {
  return currentOs() === "mac";
}

const DESKTOP: Record<Exclude<Os, "mobile">, { url: string; name: string }> = {
  mac: { url: MAC_ZIP, name: "macOS" },
  windows: { url: WIN_ZIP, name: "Windows" },
  linux: { url: LINUX_ZIP, name: "Linux" },
};

function personalDownload(): { url: string; label: string; altHtml: string; mobile: boolean } {
  const os = currentOs();
  const links = (skip?: string) => Object.entries(DESKTOP).filter(([k]) => k !== skip).map(([, d]) => `<a href="${d.url}">${d.name}</a>`).join(" · ");
  if (os === "mobile") return { url: "#roadmap", label: "Email me the download link", altHtml: `Desktop app: ${links()}`, mobile: true };
  return { url: DESKTOP[os].url, label: `Download for ${DESKTOP[os].name} — Free`, altHtml: `Also: ${links(os)}`, mobile: false };
}

// Phones get the email form (interest=download) instead of a zip they cannot run.
function applyDownload(a: HTMLAnchorElement, dl: ReturnType<typeof personalDownload>): void {
  a.textContent = dl.label;
  a.href = dl.url;
  if (dl.mobile) { a.removeAttribute("target"); a.dataset.interest = "download"; }
  else { a.target = "_blank"; delete a.dataset.interest; }
}

function initDownloadLinks(): void {
  const roadmap = document.getElementById("cta-roadmap") as HTMLAnchorElement | null;
  if (roadmap) applyDownload(roadmap, personalDownload());
}

// Single source for the default hero copy (R18.9): the personal text is
// authored in index.html, so the first paint is never swapped and crawlers
// index the same sentence users read. Captured before any track switch - a
// later switch would leave the DOM holding that track's text instead.
let personalCopy: { title: string; sub: string; proof: string } | null = null;

function setTrack(t: Track) {
  const base = copy[t];
  const c = t === "personal" && personalCopy ? { ...base, ...personalCopy } : base;
  const title = document.getElementById("hero-title");
  const sub = document.getElementById("hero-sub");
  const proof = document.getElementById("track-proof");
  const roadmap = document.getElementById("roadmap-track");
  const cta = document.getElementById("cta-primary") as HTMLAnchorElement | null;
  const note = document.getElementById("cta-note");
  if (title) title.textContent = c.title;
  if (sub) sub.textContent = c.sub;
  if (proof) proof.textContent = c.proof;
  if (roadmap) roadmap.textContent = base.roadmap;
  if (cta) {
    delete cta.dataset.interest;
    if (t === "team") {
      // Mirror the Team pricing card action (R18.5) - one target for the hero
      // CTA and the card, single source is the card link in index.html.
      const card = document.getElementById("cta-team") as HTMLAnchorElement | null;
      cta.textContent = card?.textContent?.trim() ?? "Talk to us — Team license";
      cta.href = card?.getAttribute("href") ?? "#pricing";
    } else if (t === "hosted") {
      cta.textContent = "Join waitlist — Hosted (soon)";
      cta.href = "#roadmap";
      cta.dataset.interest = "hosted";
      cta.removeAttribute("target");
    } else {
      applyDownload(cta, personalDownload());
    }
  }
  const firstLaunch = document.getElementById("first-launch");
  if (firstLaunch) firstLaunch.hidden = t !== "personal";
  if (note) {
    if (t === "team") note.innerHTML = "Docker • one container • your VPC • one-time license, incl. a year of support";
    else if (t === "hosted") note.innerHTML = "Coming soon — we host it, no setup · <code>hosted</code> tab";
    else { const dl = personalDownload(); note.innerHTML = `macOS, Windows & Linux • one download, no Docker • data stays on your machine · ${dl.altHtml}`; }
  }

  document.querySelectorAll<HTMLButtonElement>(".track-toggle .toggle").forEach(b => {
    const isActive = b.dataset.track === t;
    b.classList.toggle("active", isActive);
b.setAttribute("aria-pressed", String(isActive));
  });

  // persist for app hint (?track=) + reload
try { localStorage.setItem("anchorcore.track", t); } catch { /* private mode; ?track= still carries it */ }
  const url = new URL(location.href);
  url.searchParams.set("track", t);
  history.replaceState(null, "", url.toString());
}

function initTrack() {
  const params = new URL(location.href).searchParams.get("track") as Track | null;
  const stored = (() => { try { return localStorage.getItem("anchorcore.track") as Track | null; } catch { return null; } })();
  const valid: Track[] = ["personal", "team", "hosted"];
  const initial: Track = (params && valid.includes(params) ? params : null) ?? (stored && valid.includes(stored) ? stored : null) ?? "personal";
  // default to personal for faster validation (design-system.md:128)
  // capture the authored copy once, before any switch can rewrite it (R18.9)
  const text = (id: string, fallback: string) => document.getElementById(id)?.textContent?.trim() || fallback;
  personalCopy = {
    title: text("hero-title", copy.personal.title),
    sub: text("hero-sub", copy.personal.sub),
    proof: text("track-proof", copy.personal.proof),
  };
  setTrack(initial);
  document.querySelectorAll<HTMLButtonElement>(".track-toggle .toggle").forEach(b => {
    b.addEventListener("click", () => setTrack(b.dataset.track as Track));
  });
}

// email capture via Formspree (Vanilla JS Ajax — plain fetch, no SDK needed:
// this page has no runtime deps and the form carries interest metadata).
const FORMSPREE_ID = "mdeozqge";

type Interest = "hosted" | "team" | "updates" | "download";
let interest: Interest = "updates"; // set by the entry point that led to the form

document.addEventListener("click", (e) => {
  const a = (e.target as Element | null)?.closest<HTMLElement>("[data-interest]");
  if (a?.dataset.interest) interest = a.dataset.interest as Interest;
});

const THANKS: Record<Interest, string> = {
  hosted: "Thanks — you're on the hosted waitlist.",
  team: "Thanks — we'll be in touch about Team.",
  updates: "Thanks — we'll email you product updates.",
  download: "Thanks — we'll email you the download link.",
};

async function captureEmail(inputId: string, msgId: string, source: string) {
  const el = document.getElementById(inputId) as HTMLInputElement | null;
  const msg = document.getElementById(msgId);
  const email = el?.value.trim() ?? "";
  if (!email || !/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email)) {
    if (msg) msg.textContent = "Enter a valid email address.";
    el?.setAttribute("aria-invalid", "true");
    return;
  }
  el?.removeAttribute("aria-invalid");
  if (msg) msg.textContent = "Sending…";
  try {
    const res = await fetch(`https://formspree.io/f/${FORMSPREE_ID}`, {
      method: "POST",
      headers: { "Content-Type": "application/json", "Accept": "application/json" },
      body: JSON.stringify({ email, interest, source }),
    });
    const data = (await res.json().catch(() => ({}))) as { errors?: { message: string }[] };
    if (res.ok) {
      if (msg) msg.textContent = THANKS[interest];
      if (el) el.value = "";
    } else {
      const detail = data.errors?.map((e) => e.message).join(" ");
      if (msg) msg.textContent = detail || "Something went wrong — try again.";
      el?.setAttribute("aria-invalid", "true");
    }
  } catch {
    if (msg) msg.textContent = "Couldn't reach the signup service — check your connection and retry.";
  }
}

// On-site guides — repo markdown is the source of truth (../docs), rendered
// here at build time so conversion pages live on-domain, not on GitHub.
function stripH1(md: string): string {
  return md.replace(/^# .*\n/, "");
}

async function initGuides(): Promise<void> {
  const pairs: [string, string][] = [
    ["guide-obsidian-body", obsidianMd],
    ["guide-quickstart-body", quickstartMd],
    ["guide-rest-body", restMd],
    ["guide-ai-body", aiMd],
  ];
  for (const [id, md] of pairs) {
    const el = document.getElementById(id);
    if (!el) continue;
    // headings are demoted two levels so they nest under the panel <h3>
    // instead of jumping back to h2 inside the section (R18.8)
    el.innerHTML = (await marked.parse(stripH1(md)))
      .replace(/<(\/?)h([2-4])\b/g, (_m, close: string, lvl: string) => `<${close}h${Number(lvl) + 2}`);
  }
}

// Guides / FAQ hash routing — keep paper mono/serif, no new hex (C3)
function initHashNav() {
  const navLinks = [...document.querySelectorAll<HTMLAnchorElement>(".nav a[href^='#'], .nav-mobile a[href^='#']")];
  if (navLinks.length === 0) return;
  const paint = (hash: string) => {
    navLinks.forEach(a => {
      const isActive = hash !== "" && a.getAttribute("href") === hash;
      a.style.textDecoration = isActive ? "underline" : "none";
      a.style.textUnderlineOffset = isActive ? "3px" : "";
      a.style.color = isActive ? "var(--sage)" : "var(--ink)";
      if (isActive) a.setAttribute("aria-current", "location");
      else a.removeAttribute("aria-current");
    });
  };
  // R18.8: at the top of the page nothing is "current" (this used to
  // underline "How it works" on load) and the highlight follows the section
  // actually in view instead of only changing on hashchange.
  paint(location.hash);
  window.addEventListener("hashchange", () => paint(location.hash));
  const sections = [...document.querySelectorAll<HTMLElement>("section[id]")]
    .filter(s => navLinks.some(a => a.getAttribute("href") === `#${s.id}`));
  if (sections.length === 0 || !("IntersectionObserver" in window)) return;
  const ratios = new Map<string, number>();
  const spy = new IntersectionObserver((entries) => {
    for (const e of entries) {
      const el = e.target as HTMLElement;
      ratios.set(el.id, e.isIntersecting ? e.intersectionRatio : 0);
    }
    let best = "";
    let bestRatio = 0;
    ratios.forEach((ratio, id) => { if (ratio > bestRatio) { best = id; bestRatio = ratio; } });
    paint(bestRatio > 0 ? `#${best}` : "");
  }, { threshold: [0.15, 0.4, 0.75], rootMargin: "-72px 0px -40% 0px" });
  sections.forEach(s => spy.observe(s));
}

// GraphMini — 3 nodes, Stone & Sage, no new hex (uses CSS vars)

// submit handler (real submit — Enter key works, no inline onclick)
document.getElementById("capture-form-2")?.addEventListener("submit", (e) => {
  e.preventDefault();
  void captureEmail("email2", "capture2-msg", "roadmap");
});

// Terminal animations — abbreviated sessions (demo + agent-skill sections).
// MCP shapes mirror rust/crates/anchorcore/src/mcp.rs (tools/call with
// name + arguments.question); skill shapes mirror the anchorcore-setup skill
// (local REST: health → models → sources → sync → ask + honesty flags).
// The on-page captions note both are abbreviated.
type TermLine = { cls: string; text: string; type?: boolean; pause?: number };
const TERM_LINES: TermLine[] = [
  { cls: "t-dim", text: "$ anchorcore mcp", type: true },
  { cls: "t-dim", text: "# stdio MCP mode — spawned by your harness, no browser", pause: 350 },
  { cls: "t-out", text: '→ tools/call {"name": "ask", "arguments": {"question": "Who owns the billing provider migration?"}}', type: true },
  { cls: "t-dim", text: "··· searching memory: meaning + keywords + links", pause: 650 },
  { cls: "t-in", text: "← Sarah owns the billing provider migration — parallel run starts Monday.", pause: 300 },
  { cls: "t-in", text: "← [decision] d2-billing-provider-approval · owner: Sarah · 87%" },
  { cls: "t-in", text: "← [note] m2-q3-planning · 74%" },
  { cls: "t-dim", text: "← done: 1 answer · 2 citations · logged" },
];

const SKILL_LINES: TermLine[] = [
  { cls: "t-dim", text: "$ GET /health → {\"status\": \"ok\"}", type: true },
  { cls: "t-out", text: "→ GET /system/status · answer_ready: false · missing: embed_model" },
  { cls: "t-dim", text: "$ POST /system/ollama/pull {\"models\": [\"llama3.2:3b\", \"nomic-embed-text\"]}", pause: 350 },
  { cls: "t-dim", text: "··· pulling models · two free downloads, then local forever", pause: 650 },
  { cls: "t-dim", text: "$ POST /sources {\"connector\": \"folder\", \"name\": \"Sample\"} → id 3" },
  { cls: "t-dim", text: "$ POST /sources/3/sync → done · about 40 entities filed", pause: 400 },
  { cls: "t-out", text: "→ POST /qa {\"question\": \"Who owns the billing provider migration?\"}", type: true },
  { cls: "t-in", text: "← Sarah owns the billing provider migration — parallel run starts Monday.", pause: 300 },
  { cls: "t-in", text: "← [decision] d2-billing-provider-approval · 87% · mode=composed" },
  { cls: "t-dim", text: "← done: cited answer · 2 citations · local-only" },
];

function termLine(el: HTMLElement, cls: string, text: string): void {
  const div = document.createElement("div");
  div.className = cls;
  div.textContent = text;
  el.appendChild(div);
}

function initTerminal(bodyId: string, replayId: string, lines: TermLine[]): void {
  const body = document.getElementById(bodyId);
  const replay = document.getElementById(replayId);
  if (!body) return;
  // Typed output is decorative; screen readers get the full transcript once (R44.8)
  body.setAttribute("aria-hidden", "true");
  const transcript = document.createElement("div");
  transcript.className = "sr-only";
  transcript.textContent = lines.map((l) => l.text).join("\n");
  body.after(transcript);
  const reduced = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  let started = false;
  let run = 0;

  function renderAll(): void {
    body!.innerHTML = "";
    for (const line of lines) termLine(body!, line.cls, line.text);
  }

  async function play(): Promise<void> {
    const my = ++run;
    body!.innerHTML = "";
    const caret = document.createElement("span");
    caret.className = "caret";
    for (const line of lines) {
      if (line.pause) await new Promise((r) => setTimeout(r, line.pause));
      if (my !== run) return;
      if (line.type && !reduced) {
        const div = document.createElement("div");
        div.className = line.cls;
        body!.appendChild(div);
        div.appendChild(caret);
        for (let i = 1; i <= line.text.length; i++) {
          div.insertBefore(document.createTextNode(line.text[i - 1]), caret);
          await new Promise((r) => setTimeout(r, 14));
          if (my !== run) return;
        }
        caret.remove();
      } else {
        termLine(body!, line.cls, line.text);
      }
    }
  }

  function start(): void {
    if (started) return;
    started = true;
    if (reduced) renderAll();
    else void play();
  }

  replay?.addEventListener("click", () => {
    if (reduced) renderAll();
    else void play();
  });

  if ("IntersectionObserver" in window) {
    const obs = new IntersectionObserver((entries) => {
      if (entries.some((e) => e.isIntersecting)) {
        start();
        obs.disconnect();
      }
    }, { threshold: 0.3 });
    obs.observe(body);
  } else {
    start();
  }
}


// Second corpus (R18.15): ?corpus=ai swaps the landing examples. The payments
// examples stay in the markup as the default, so this is progressive
// enhancement on top of static HTML rather than the source of truth. The task
// text said ?track=, but that has been the Personal/Team/Hosted switch since
// R18.1, so the corpus gets its own parameter (see website/README.md).
const AI_SHOT = {
  q: "What did we decide about model routing, and why?",
  a: "Tiered routing (router v2) — short lookups stay on small-local, multi-step reasoning can reach frontier-api only when the tenant's egress policy allows it. 62% less spend on the lookup slice at the same answer quality, owned by Noor.",
  cites: [
    ["sage", "Model router v2 (tiered routing)", "decision · approved", "“Frontier spend grew 3.1x while answer quality on the reasoning slice was flat.”", "d1-model-router-v2.md · decision"],
    ["stone", "Postmortem — prompts reached the cloud provider", "incident · owner: Noor", "“The router now derives the tier server-side and ignores client hints for anything policy-relevant.”", "m2-incident-postmortem.md · meeting notes"],
  ],
  foot: "2 citations · answered on your machine",
};
const AI_DEMO = [
  "<b>Connect</b> the <code>sample-ai/</code> demo folder — a fictional AI company's routing, eval and pricing decisions.",
  "Ask: <code>What did we decide about model routing, and why?</code> → the tiered-routing decision, cited.",
  "Follow-up: <code>what did we change after the incident?</code> → three changes, each with an owner.",
  "Ask: <code>What is our citation accuracy?</code> → <code>0.89</code> on the rebuilt holdout set, with the correction explained.",
];

function initCorpus(): void {
  if (new URL(location.href).searchParams.get("corpus") !== "ai") return;
  const shot = document.querySelector(".shot-body");
  if (shot) {
    shot.innerHTML = `<div class="q">${AI_SHOT.q}</div><div class="a">${AI_SHOT.a}</div><div class="cites">` +
      AI_SHOT.cites.map(([tone, title, meta, quote, src]) =>
        `<div class="cite"><span class="bar ${tone}"></span><div><b>${title}</b> <small>${meta}</small><p>${quote}</p><small>${src}</small></div></div>`).join("") +
      `</div>`;
    const foot = document.querySelector(".shot-foot");
    if (foot) foot.textContent = AI_SHOT.foot;
  }
  const demo = document.querySelector("ol.demo");
  if (demo) demo.innerHTML = AI_DEMO.map(li => `<li>${li}</li>`).join("");
}

function initGuideTabs(): void {
  const tabs = [...document.querySelectorAll<HTMLButtonElement>(".guide-tab")];
  if (tabs.length === 0) return;
  // derive the pairings from the markup so a new tab needs no change here (R18.15)
  const bodies = tabs.map(t => t.dataset.guide ?? "");
  const panels = tabs.map(t => document.getElementById(t.getAttribute("aria-controls") ?? ""));
  function show(index: number): void {
    tabs.forEach((t, i) => {
      const on = i === index;
      t.classList.toggle("active", on);
t.setAttribute("aria-expanded", String(on));
    });
    panels.forEach((p, i) => {
      if (!p) return;
      const on = i === index;
      p.classList.toggle("active", on);
      if (on) p.removeAttribute("hidden");
      else p.setAttribute("hidden", "");
    });
  }
  tabs.forEach((t, i) => t.addEventListener("click", () => show(i)));
  // deep links (#guide-obsidian etc.) open the right panel first (R18.7: one section)
  const hash = location.hash;
  const legacy = bodies.indexOf(hash.slice(1) + "-body");
  if (legacy >= 0) show(legacy);
  window.addEventListener("hashchange", () => {
    const i = bodies.indexOf(location.hash.slice(1) + "-body");
    if (i >= 0) {
      show(i);
      document.getElementById("guide-reader")?.scrollIntoView({ behavior: "smooth" });
    }
  });
}

// Mobile nav (R18.1): <details> works without JS; close it on link click, outside click, Escape, desktop resize.
function initNavMenu(): void {
  const menu = document.getElementById("navmenu") as HTMLDetailsElement | null;
  if (!menu) return;
  const close = (): void => { menu.open = false; };
  menu.addEventListener("click", (e) => { if ((e.target as HTMLElement).closest("a")) close(); });
  document.addEventListener("click", (e) => { if (menu.open && !menu.contains(e.target as Node)) close(); });
  document.addEventListener("keydown", (e) => { if (e.key === "Escape") close(); });
  window.matchMedia("(min-width: 761px)").addEventListener("change", close);
}

// Hero agent-skill card: copy the install command (R18.19). No JS: the code stays
// visible and selectable, only the one-click copy is lost.
function selectText(el: HTMLElement): void {
  const range = document.createRange();
  range.selectNodeContents(el);
  const sel = window.getSelection();
  sel?.removeAllRanges();
  sel?.addRange(range);
}

function initSkillCopy(): void {
  const btn = document.getElementById("skill-copy");
  const cmd = document.getElementById("skill-cmd");
  if (!btn || !cmd) return;
  const label = btn.textContent ?? "Copy";
  const flash = (text: string): void => {
    btn.textContent = text;
    setTimeout(() => { btn.textContent = label; }, 1800);
  };
  btn.addEventListener("click", async () => {
    try {
      await navigator.clipboard.writeText(cmd.textContent ?? "");
      flash("Copied");
      return;
    } catch { /* insecure context or clipboard permission denied */ }
    // fallback: select the command and try the legacy path, so Ctrl+C always works
    selectText(cmd);
    const legacy = (() => { try { return document.execCommand("copy"); } catch { return false; } })();
    flash(legacy ? "Copied" : (isMacOs() ? "Press ⌘C" : "Press Ctrl+C"));
  });
}

// Airy scroll reveal — one observer, no deps
function initReveal(): void {
  if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;
  // the hero is above the fold: revealing it delays first paint (R44.3)
  const targets = Array.from(document.querySelectorAll("section:not(.hero) .wrap, .col, .step, .qa, .fig, .flow")).filter((el) => !el.closest(".hero"));
  targets.forEach((el) => el.classList.add("reveal"));
  if (!("IntersectionObserver" in window)) {
    targets.forEach((el) => el.classList.add("in"));
    return;
  }
  const obs = new IntersectionObserver(
    (entries) => {
      for (const e of entries) {
        if (e.isIntersecting) {
          e.target.classList.add("in");
          obs.unobserve(e.target);
        }
      }
    },
    { threshold: 0.12 },
  );
  targets.forEach((el) => obs.observe(el));
}

initTrack();
initDownloadLinks();
initHashNav();
initNavMenu();
initSkillCopy();
void initGuides();
initGuideTabs();
initCorpus();
initReveal();
initTerminal("term-body", "term-replay", TERM_LINES);
initTerminal("skill-term-body", "skill-term-replay", SKILL_LINES);
// render after DOM ready (both graphs are JS-built; R18.6 keeps them in sync)
const renderGraphs = () => {
  renderGraphMini("graph-mini");
  renderGraphMini("graph-mini-2");
};
if (document.readyState === "loading") {
  document.addEventListener("DOMContentLoaded", renderGraphs);
} else {
  renderGraphs();
}
