// Auto / Light / Dark. The choice lives in localStorage ("theme-mode");
// "auto" follows the OS. The resolved theme is written to
// <html data-theme="light|dark">, which theme.css keys the dark tokens on.
// index.html runs the same resolution inline before first paint (no flash) —
// keep the two in sync.

export type ThemeMode = "auto" | "light" | "dark";

const KEY = "theme-mode";
const DARK_QUERY = "(prefers-color-scheme: dark)";

export function parseMode(v: string | null): ThemeMode {
  return v === "light" || v === "dark" ? v : "auto";
}

export function resolveTheme(mode: ThemeMode, prefersDark: boolean): "light" | "dark" {
  return mode === "auto" ? (prefersDark ? "dark" : "light") : mode;
}

export function readMode(): ThemeMode {
  try {
    return parseMode(localStorage.getItem(KEY));
  } catch {
    return "auto"; // storage blocked: follow the OS
  }
}

function prefersDark(): boolean {
  return window.matchMedia?.(DARK_QUERY).matches ?? false;
}

export function applyMode(mode: ThemeMode): void {
  document.documentElement.dataset.theme = resolveTheme(mode, prefersDark());
  try {
    if (mode === "auto") localStorage.removeItem(KEY);
    else localStorage.setItem(KEY, mode);
  } catch {
    /* not persisted; still applied for this session */
  }
}

/** Re-resolves "auto" when the OS theme flips. Returns an unsubscribe. */
export function followSystem(getMode: () => ThemeMode): () => void {
  const mq = window.matchMedia?.(DARK_QUERY);
  if (!mq) return () => {};
  const onChange = () => {
    if (getMode() === "auto") document.documentElement.dataset.theme = resolveTheme("auto", mq.matches);
  };
  mq.addEventListener("change", onChange);
  return () => mq.removeEventListener("change", onChange);
}
