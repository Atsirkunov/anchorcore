// Auto / Light / Dark — same contract as the app (frontend/src/themeMode.ts):
// the choice lives in localStorage ("theme-mode"), "auto" follows the OS, and
// the resolved theme is written to <html data-theme="light|dark">, which
// style.css keys the dark tokens on. Every page's <head> runs the same
// resolution inline before first paint (no flash) — keep them in sync.
// The switch is injected here so the eleven copied headers stay untouched;
// without JS there is no switch and the page follows the OS via that script.

type ThemeMode = "auto" | "light" | "dark";

const KEY = "theme-mode";
const DARK = "(prefers-color-scheme: dark)";
const MODES: { id: ThemeMode; label: string; title: string }[] = [
  { id: "auto", label: "Auto", title: "Follow the system setting" },
  { id: "light", label: "Light", title: "Always light" },
  { id: "dark", label: "Dark", title: "Always dark" },
];

function readMode(): ThemeMode {
  try {
    const v = localStorage.getItem(KEY);
    return v === "light" || v === "dark" ? v : "auto";
  } catch {
    return "auto";
  }
}

function resolve(mode: ThemeMode): "light" | "dark" {
  if (mode !== "auto") return mode;
  return window.matchMedia?.(DARK).matches ? "dark" : "light";
}

function apply(mode: ThemeMode): void {
  document.documentElement.dataset.theme = resolve(mode);
  try {
    if (mode === "auto") localStorage.removeItem(KEY);
    else localStorage.setItem(KEY, mode);
  } catch {
    /* applied for this visit only */
  }
}

function build(extraClass: string): HTMLElement {
  const group = document.createElement("div");
  group.className = `theme-switch ${extraClass}`;
  group.setAttribute("role", "group");
  group.setAttribute("aria-label", "Theme");
  for (const m of MODES) {
    const b = document.createElement("button");
    b.type = "button";
    b.textContent = m.label;
    b.title = m.title;
    b.dataset.mode = m.id;
    group.appendChild(b);
  }
  return group;
}

export function initThemeSwitch(): void {
  const header = document.querySelector(".site-header .wrap");
  if (!header) return;
  // one in the header (desktop), one at the end of the mobile menu (phones)
  const groups = [build("in-header"), build("in-menu")];
  header.querySelector(".nav-cta")?.before(groups[0]);
  document.querySelector(".nav-mobile")?.appendChild(groups[1]);
  let mode = readMode();
  const paint = (): void => {
    for (const b of document.querySelectorAll<HTMLButtonElement>(".theme-switch button")) {
      b.setAttribute("aria-pressed", String(b.dataset.mode === mode));
    }
  };
  for (const g of groups) {
    g.addEventListener("click", (e) => {
      const b = (e.target as HTMLElement).closest<HTMLButtonElement>("button[data-mode]");
      if (!b) return;
      mode = b.dataset.mode as ThemeMode;
      apply(mode);
      paint();
    });
  }
  window.matchMedia?.(DARK).addEventListener("change", () => {
    if (mode === "auto") document.documentElement.dataset.theme = resolve("auto");
  });
  paint();
}
