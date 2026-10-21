import { renderGraphMini } from "./components/GraphMini";
import { initClaudeSession } from "./components/ClaudeSession";
import { initEmailCapture } from "./capture";
import { initNavMenu } from "./nav";

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
  return { url: DESKTOP[os].url, label: "Download — free for personal use", altHtml: `Also: ${links(os)}`, mobile: false };
}

// Phones get the email form (interest=download) instead of a zip they cannot run.
function applyDownload(a: HTMLAnchorElement, dl: ReturnType<typeof personalDownload>): void {
  a.textContent = dl.label;
  a.href = dl.url;
  if (dl.mobile) { a.removeAttribute("target"); a.dataset.interest = "download"; }
  else { a.target = "_blank"; delete a.dataset.interest; }
}

function initDownloadLinks(): void {
  const dl = personalDownload();
  for (const id of ["cta-primary", "cta-roadmap"]) {
    const a = document.getElementById(id) as HTMLAnchorElement | null;
    if (a) applyDownload(a, dl);
  }
  const note = document.getElementById("cta-note");
  if (note) note.innerHTML = `macOS, Windows & Linux • one download, no Docker • data stays on your machine · ${dl.altHtml}`;
}

// Audience pages replaced the hero toggle (R44.16): old ?track= links still land
// on the right page. ?track=personal (and anything else) is a no-op.
const TRACK_PAGES: Record<string, string> = { team: "/team", hosted: "/hosted" };

function redirectLegacyTrack(): boolean {
  const target = TRACK_PAGES[new URL(location.href).searchParams.get("track") ?? ""];
  if (!target) return false;
  location.replace(target);
  return true;
}

// Guides are prerendered pages (/guides/*, R44.15). Old #guide-* deep links jump
// to the matching page; without JS they still land on the card with that id.
const GUIDE_PAGES: Record<string, string> = {
  "guide-obsidian": "/guides/obsidian",
  "guide-quickstart": "/guides/sample",
  "guide-rest": "/guides/json-api",
  "guide-ai": "/guides/ai-startup",
  "guide-mcp": "/guides/mcp",
};

function redirectLegacyGuide(): boolean {
  const target = GUIDE_PAGES[location.hash.slice(1)];
  if (!target) return false;
  location.replace(target);
  return true;
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
  { cls: "t-dim", text: "··· searching your files: meaning + keywords + links", pause: 650 },
  { cls: "t-in", text: "← Sarah owns the billing provider migration — parallel run starts Monday.", pause: 300 },
  { cls: "t-in", text: "← [decision] d2-billing-provider-approval · owner: Sarah" },
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
  { cls: "t-in", text: "← [decision] d2-billing-provider-approval · mode=composed" },
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

initEmailCapture();
if (!redirectLegacyTrack() && !redirectLegacyGuide()) initDownloadLinks();
initHashNav();
initNavMenu();
initSkillCopy();
initReveal();
initClaudeSession("cs-hero");
initClaudeSession("cs-demo");
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
