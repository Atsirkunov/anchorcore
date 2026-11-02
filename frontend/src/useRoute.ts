import { useCallback, useEffect, useState } from "react";

// The open tab (and section) live in the URL hash, so reload, back/forward and
// links work (R49.1). R49.9 layout: Ask · Library · Records · Agents ·
// Settings; `#/library/<source id>` opens a source, `#/settings/<section>` a
// Settings section. Hash routes need nothing from the server: every path
// still serves the same SPA shell.

export const TABS = ["ask", "library", "records", "agents", "settings"] as const;
export type Tab = (typeof TABS)[number];

export const SETTINGS_SECTIONS = [
  "answers",
  "search",
  "sharing",
  "glossary",
  "backup",
  "updates",
  "status",
  "appearance",
  "advanced",
] as const;
export type SettingsSection = (typeof SETTINGS_SECTIONS)[number];

export type Route = { tab: Tab; sub: string | null };

/** Hashes from before R49.9 (bookmarks, notices) and where they live now. */
const ALIASES: Record<string, Route> = {
  sources: { tab: "library", sub: null },
  system: { tab: "settings", sub: "status" },
};

/** The route a hash names; anything unknown (or empty) is Ask. */
export function parseRoute(hash: string): Route {
  const parts = hash.replace(/^#\/?/, "").split("?")[0].split("/").map((p) => p.toLowerCase());
  const head = parts[0] ?? "";
  if (ALIASES[head]) return ALIASES[head];
  if (!(TABS as readonly string[]).includes(head)) return { tab: "ask", sub: null };
  return { tab: head as Tab, sub: parts[1] || null };
}

/** The Settings section a route opens (defaults to Answers). */
export function settingsSection(sub: string | null): SettingsSection {
  return (SETTINGS_SECTIONS as readonly string[]).includes(sub ?? "") ? (sub as SettingsSection) : "answers";
}

export function routeHash(tab: Tab, sub?: string | number | null): string {
  return sub === undefined || sub === null || sub === "" ? `#/${tab}` : `#/${tab}/${sub}`;
}

/** The current route and a setter that pushes a history entry. */
export function useRoute(): [Route, (tab: Tab, sub?: string | number | null) => void] {
  const [route, setRouteState] = useState<Route>(() => parseRoute(window.location.hash));
  useEffect(() => {
    const onChange = () => setRouteState(parseRoute(window.location.hash));
    window.addEventListener("hashchange", onChange);
    return () => window.removeEventListener("hashchange", onChange);
  }, []);
  const navigate = useCallback((tab: Tab, sub?: string | number | null) => {
    const hash = routeHash(tab, sub);
    if (window.location.hash !== hash) window.location.hash = hash;
    setRouteState(parseRoute(hash));
  }, []);
  return [route, navigate];
}
