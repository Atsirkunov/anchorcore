// Mobile nav (R18.1): <details> works without JS; close it on link click, outside click, Escape, desktop resize.
export function initNavMenu(): void {
  const menu = document.getElementById("navmenu") as HTMLDetailsElement | null;
  if (!menu) return;
  const close = (): void => { menu.open = false; };
  menu.addEventListener("click", (e) => { if ((e.target as HTMLElement).closest("a")) close(); });
  document.addEventListener("click", (e) => { if (menu.open && !menu.contains(e.target as Node)) close(); });
  document.addEventListener("keydown", (e) => { if (e.key === "Escape") close(); });
  window.matchMedia("(min-width: 761px)").addEventListener("change", close);
}

