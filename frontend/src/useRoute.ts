import { useCallback, useEffect, useState } from "react";

// Wave 1 (UX audit #3): the open tab lives in the URL hash (`#/sources`), so
// reload, back/forward and links to a tab work. Hash routes need nothing from
// the server: every path still serves the same SPA shell.

export const TABS = ["ask", "sources", "records", "settings", "system"] as const;
export type Tab = (typeof TABS)[number];

/** The tab a hash names; anything unknown (or empty) is Ask. */
export function parseRoute(hash: string): Tab {
  const head = hash.replace(/^#\/?/, "").split(/[/?]/)[0].toLowerCase();
  return (TABS as readonly string[]).includes(head) ? (head as Tab) : "ask";
}

export function routeHash(tab: Tab): string {
  return `#/${tab}`;
}

/** The current tab and a setter that pushes a history entry. */
export function useRoute(): [Tab, (tab: Tab) => void] {
  const [tab, setTabState] = useState<Tab>(() => parseRoute(window.location.hash));
  useEffect(() => {
    const onChange = () => setTabState(parseRoute(window.location.hash));
    window.addEventListener("hashchange", onChange);
    return () => window.removeEventListener("hashchange", onChange);
  }, []);
  const setTab = useCallback((next: Tab) => {
    if (window.location.hash !== routeHash(next)) window.location.hash = routeHash(next);
    setTabState(next);
  }, []);
  return [tab, setTab];
}
