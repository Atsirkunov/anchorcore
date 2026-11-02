// Light / Auto / Dark. R49.7 (owner decision 2026-11-02): the site opens in
// Light, the designed palette; "auto" follows the OS and "dark" forces dark.
// The choice lives in localStorage ("theme-mode", absent = light) and the
// resolved theme is written to <html data-theme="light|dark">, which style.css
// keys the dark tokens on. Every page's <head> (and scripts/build-guides.mjs)
// runs the same resolution inline before first paint — keep them in sync.
// The switch is injected into the footer so the copied pages stay untouched;
// without JS there is no switch and the page stays Light unless stored.

type ThemeMode = "auto" | "light" | "dark";

const KEY = "theme-mode";
const DARK = "(prefers-color-scheme: dark)";
const MODES: { id: ThemeMode; label: string; title: string }[] = [
  { id: "light", label: "Light", title: "Always light (default)" },
  { id: "auto", label: "Auto", title: "Follow the system setting" },
  { id: "dark", label: "Dark", title: "Always dark" },
];

function readMode(): ThemeMode {
  try {
    const v = localStorage.getItem(KEY);
    return v === "auto" || v === "dark" ? v : "light";
  } catch {
    return "light";
  }
}

function resolve(mode: ThemeMode): "light" | "dark" {
  if (mode !== "auto") return mode;
  return window.matchMedia?.(DARK).matches ? "dark" : "light";
}

function apply(mode: ThemeMode): void {
  document.documentElement.dataset.theme = resolve(mode);
  try {
    if (mode === "light") localStorage.removeItem(KEY);
    else localStorage.setItem(KEY, mode);
  } catch {
    /* applied for this visit only */
  }
}

function build(): HTMLElement {
  const group = document.createElement("div");
  group.className = "theme-switch";
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
  const foot = document.querySelector("footer .foot");
  if (!foot) return;
  const groups = [build()];
  foot.appendChild(groups[0]);
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
