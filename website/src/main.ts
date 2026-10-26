import { initClaudeSession } from "./components/ClaudeSession";
import { initAppSession } from "./components/AppSession";
import { initEmailCapture } from "./capture";
import { initNavMenu } from "./nav";
import { initThemeSwitch } from "./themeMode";

// Direct download assets (release.yml — keep these file names in sync).
const WIN_ZIP = "https://github.com/Atsirkunov/anchorcore/releases/latest/download/AnchorCore-windows.zip";
// macOS ships a drag-to-Applications DMG (R41.2); the .zip is still attached for
// one release but the site never links it. Needs a release built by the R41.2
// release.yml: deploy the site only together with (or after) that tag.
const MAC_DMG = "https://github.com/Atsirkunov/anchorcore/releases/latest/download/AnchorCore-macos.dmg";

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
  mac: { url: MAC_DMG, name: "macOS" },
  windows: { url: WIN_ZIP, name: "Windows" },
  linux: { url: LINUX_ZIP, name: "Linux" },
};

function personalDownload(): { url: string; label: string; altHtml: string; mobile: boolean } {
  const os = currentOs();
  const links = (skip?: string) => Object.entries(DESKTOP).filter(([k]) => k !== skip).map(([, d]) => `<a href="${d.url}">${d.name}</a>`).join(" · ");
  if (os === "mobile") return { url: "#roadmap", label: "Email me the download link", altHtml: `Desktop app: ${links()}`, mobile: true };
  return { url: DESKTOP[os].url, label: "Download, free for personal use", altHtml: `Also: ${links(os)}`, mobile: false };
}

// Phones get the email form (interest=download) instead of a download they cannot run.
function applyDownload(a: HTMLAnchorElement, dl: ReturnType<typeof personalDownload>): void {
  a.textContent = dl.label;
  a.href = dl.url;
  if (dl.mobile) { a.removeAttribute("target"); a.dataset.interest = "download"; }
  else { a.target = "_blank"; delete a.dataset.interest; }
}

function initDownloadLinks(): void {
  const dl = personalDownload();
  for (const id of ["cta-roadmap"]) {
    const a = document.getElementById(id) as HTMLAnchorElement | null;
    if (a) applyDownload(a, dl);
  }
  const note = document.getElementById("cta-note");
  if (note) note.innerHTML = `macOS, Windows and Linux. One download, no Docker, no account. ${dl.altHtml}`;
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
      a.style.color = isActive ? "var(--cite)" : "var(--ink)";
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


// Agent card: copy the skill install command (R18.19). No JS: the code stays
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

// Hero demo: one question, two places to ask it (coding agent | browser app).
// Without JS the agent panel shows; the app panel stays in the DOM, hidden.
function initDemoTabs(onShow: Record<string, () => void>): void {
  const tabs = [...document.querySelectorAll<HTMLButtonElement>(".demo-switch [role=tab]")];
  const select = (tab: HTMLButtonElement, focus: boolean): void => {
    for (const t of tabs) {
      const on = t === tab;
      t.setAttribute("aria-selected", String(on));
      t.tabIndex = on ? 0 : -1;
      const panel = document.getElementById(t.getAttribute("aria-controls") ?? "");
      if (panel) panel.hidden = !on;
      if (on && panel) onShow[panel.id]?.();
    }
    if (focus) tab.focus();
  };
  tabs.forEach((tab, i) => {
    tab.addEventListener("click", () => select(tab, false));
    tab.addEventListener("keydown", (e) => {
      const step = e.key === "ArrowRight" ? 1 : e.key === "ArrowLeft" ? -1 : 0;
      if (step) select(tabs[(i + step + tabs.length) % tabs.length], true);
    });
  });
}

// Scroll reveal, an enhancement only: content is visible unless this runs, and
// it never runs for reduced motion or automation (headless capture, crawlers
// that execute JS but never scroll). Printing reveals everything.
function initReveal(): void {
  if (window.matchMedia("(prefers-reduced-motion: reduce)").matches || navigator.webdriver) return;
  if (!("IntersectionObserver" in window)) return;
  // the hero is above the fold: revealing it delays first paint (R44.3)
  const targets = Array.from(document.querySelectorAll("section:not(.hero) .wrap, .col, .step, .qa, .fig")).filter((el) => !el.closest(".hero"));
  targets.forEach((el) => el.classList.add("reveal"));
  window.addEventListener("beforeprint", () => targets.forEach((el) => el.classList.add("in")));
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
initThemeSwitch();
initSkillCopy();
initReveal();
initClaudeSession("cs-hero");
initDemoTabs({ "panel-app": initAppSession("ap-hero") });
