import { renderGraphMini } from "./components/GraphMini";
import { marked } from "marked";
import obsidianMd from "../../docs/guides/obsidian-vault.md?raw";
import quickstartMd from "../../docs/sample-dataset.md?raw";
import restMd from "../../docs/guides/rest-api.md?raw";

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

function isMacOs(): boolean {
  const platform = navigator.userAgentData?.platform ?? navigator.platform ?? "";
  return /mac/i.test(platform);
}

function personalDownload(): { url: string; label: string; altUrl: string; altLabel: string } {
  return isMacOs()
    ? { url: MAC_ZIP, label: "Download for macOS — Free", altUrl: WIN_ZIP, altLabel: "Windows" }
    : { url: WIN_ZIP, label: "Download for Windows — Free", altUrl: MAC_ZIP, altLabel: "macOS" };
}

function initDownloadLinks(): void {
  const roadmap = document.getElementById("cta-roadmap") as HTMLAnchorElement | null;
  if (roadmap) {
    const dl = personalDownload();
    roadmap.textContent = dl.label;
    roadmap.href = dl.url;
  }
}

function setTrack(t: Track) {
  const c = copy[t];
  const title = document.getElementById("hero-title");
  const sub = document.getElementById("hero-sub");
  const proof = document.getElementById("track-proof");
  const roadmap = document.getElementById("roadmap-track");
  const cta = document.getElementById("cta-primary") as HTMLAnchorElement | null;
  const note = document.getElementById("cta-note");
  if (title) title.textContent = c.title;
  if (sub) sub.textContent = c.sub;
  if (proof) proof.textContent = c.proof;
  if (roadmap) roadmap.textContent = c.roadmap;
  if (cta) {
    if (t === "team") {
      // Mirror the Team pricing card action (R18.5) - one target for the hero
      // CTA and the card, single source is the card link in index.html.
      const card = document.getElementById("cta-team") as HTMLAnchorElement | null;
      cta.textContent = card?.textContent?.trim() ?? "Talk to us — Team license";
      cta.href = card?.getAttribute("href") ?? "#pricing";
    } else if (t === "hosted") {
      cta.textContent = "Join waitlist — Hosted (soon)";
      cta.href = "#roadmap";
    } else {
      const dl = personalDownload();
      cta.textContent = dl.label;
      cta.href = dl.url;
    }
  }
  if (note) {
    if (t === "team") note.innerHTML = "Docker • one container • your VPC • one-time license, incl. a year of support";
    else if (t === "hosted") note.innerHTML = "Coming soon — we host it, no setup · <code>hosted</code> tab";
    else { const dl = personalDownload(); note.innerHTML = `macOS & Windows • one download, no Docker • data stays on your machine · <a href="${dl.altUrl}">Download for ${dl.altLabel}</a>`; }
  }

  document.querySelectorAll<HTMLButtonElement>(".track-toggle .toggle").forEach(b => {
    const isActive = b.dataset.track === t;
    b.classList.toggle("active", isActive);
    b.setAttribute("aria-selected", String(isActive));
  });

  // persist for app hint (?track=) + reload
  try { localStorage.setItem("anchorcore.track", t); } catch {}
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
  setTrack(initial);
  document.querySelectorAll<HTMLButtonElement>(".track-toggle .toggle").forEach(b => {
    b.addEventListener("click", () => setTrack(b.dataset.track as Track));
  });
}

// email capture via Formspree (Vanilla JS Ajax — plain fetch, no SDK needed:
// this page has no runtime deps and the two forms carry track metadata).
const FORMSPREE_ID = "mdeozqge";

async function captureEmail(inputId: string, msgId: string, source: string) {
  const el = document.getElementById(inputId) as HTMLInputElement | null;
  const msg = document.getElementById(msgId);
  const email = el?.value.trim() ?? "";
  if (!email || !/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email)) {
    if (msg) msg.textContent = "Enter a valid work email.";
    return;
  }
  const track = (() => { try { return localStorage.getItem("anchorcore.track") ?? "personal"; } catch { return "personal"; } })();
  if (msg) msg.textContent = "Sending…";
  try {
    const res = await fetch(`https://formspree.io/f/${FORMSPREE_ID}`, {
      method: "POST",
      headers: { "Content-Type": "application/json", "Accept": "application/json" },
      body: JSON.stringify({ email, track, source }),
    });
    const data = await res.json().catch(() => ({} as { errors?: { message: string }[] }));
    if (res.ok) {
      if (msg) msg.textContent = `Thanks — you're on the ${track} list.`;
      if (el) el.value = "";
    } else {
      const detail = data.errors?.map((e) => e.message).join(" ");
      if (msg) msg.textContent = detail || "Something went wrong — try again.";
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
  ];
  for (const [id, md] of pairs) {
    const el = document.getElementById(id);
    if (el) el.innerHTML = await marked.parse(stripH1(md));
  }
}

// Guides / FAQ hash routing — keep paper mono/serif, no new hex (C3)
function initHashNav() {
  const navLinks = document.querySelectorAll<HTMLAnchorElement>(".nav a[href^='#'], .nav-mobile a[href^='#']");
  function setActive() {
    const hash = location.hash || "#how";
    navLinks.forEach(a => {
      const isActive = a.getAttribute("href") === hash;
      a.style.textDecoration = isActive ? "underline" : "none";
      a.style.textUnderlineOffset = isActive ? "3px" : "";
      a.style.color = isActive ? "var(--sage)" : "var(--ink)";
    });
  }
  window.addEventListener("hashchange", setActive);
  setActive();
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

function initGuidebook(): void {
  const tabs = [...document.querySelectorAll<HTMLButtonElement>(".guide-tab")];
  const panels = [
    document.getElementById("panel-obsidian"),
    document.getElementById("panel-quickstart"),
    document.getElementById("panel-rest"),
  ];
  const bodies = ["guide-obsidian-body", "guide-quickstart-body", "guide-rest-body"];
  if (tabs.length === 0) return;
  function show(index: number): void {
    tabs.forEach((t, i) => {
      const on = i === index;
      t.classList.toggle("active", on);
      t.setAttribute("aria-selected", String(on));
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
  // deep links (#guide-obsidian etc.) open the right panel first
  const hash = location.hash;
  const legacy = bodies.indexOf(hash.slice(1) + "-body");
  if (legacy >= 0) show(legacy);
  window.addEventListener("hashchange", () => {
    const i = bodies.indexOf(location.hash.slice(1) + "-body");
    if (i >= 0) {
      show(i);
      document.getElementById("guidebook")?.scrollIntoView({ behavior: "smooth" });
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

// Airy scroll reveal — one observer, no deps
function initReveal(): void {
  if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;
  const targets = document.querySelectorAll("section .wrap, .col, .step, .qa, .fig, .flow");
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
void initGuides();
initGuidebook();
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
