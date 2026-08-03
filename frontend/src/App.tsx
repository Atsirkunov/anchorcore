import { useEffect, useState } from "react";
import { api } from "./api";
import type { Health } from "./types";
import { AskTab } from "./tabs/AskTab";
import { EntitiesTab } from "./tabs/EntitiesTab";
import { ReviewTab } from "./tabs/ReviewTab";
import { SourcesTab } from "./tabs/SourcesTab";

type Tab = "ask" | "sources" | "entities" | "review";

const TABS: { id: Tab; label: string }[] = [
  { id: "ask", label: "Ask" },
  { id: "sources", label: "Sources" },
  { id: "entities", label: "Entities" },
  { id: "review", label: "Review" },
];

export default function App() {
  const [tab, setTab] = useState<Tab>("ask");
  const [online, setOnline] = useState<boolean | null>(null);
  const [health, setHealth] = useState<Health | null>(null);
  const [bannerDismissed, setBannerDismissed] = useState(false);

  useEffect(() => {
    let cancelled = false;
    async function poll() {
      try {
        const h = await api.health();
        if (cancelled) return;
        setHealth(h);
        setOnline(true);
      } catch {
        if (!cancelled) setOnline(false);
      }
    }
    poll();
    const timer = setInterval(poll, 30000);
    return () => {
      cancelled = true;
      clearInterval(timer);
    };
  }, []);

  const issues = health
    ? [
        ...(health.components.ollama === "offline" ? ["Ollama offline — classification falls back to rules, answers are limited"] : []),
        ...(health.components.answer_key === "missing" ? ["No answer model configured — answers show matching context only (set ANCHOR_ANSWER_API_KEY or point ANCHOR_ANSWER_BASE_URL at Ollama)"] : []),
        ...health.components.failing_sources.map((s) => `Sync failing: ${s.name} (${s.count}×) — ${s.error ?? "unknown error"}`),
      ]
    : [];

  return (
    <div style={styles.wrap}>
      <header style={styles.header}>
        <h1 style={styles.title}>AnchorCore</h1>
        <nav style={styles.nav}>
          {TABS.map((t) => (
            <button
              key={t.id}
              onClick={() => setTab(t.id)}
              style={{
                ...styles.tab,
                ...(tab === t.id ? styles.tabActive : {}),
              }}
            >
              {t.label}
            </button>
          ))}
        </nav>
        <span style={{ color: online === false ? "#f87171" : online ? "#4ade80" : "#6b7280", fontSize: 12 }}>
          {online === false ? "API offline" : online ? "API online" : "checking…"}
        </span>
      </header>
      {!bannerDismissed && issues.length > 0 && (
        <div style={styles.banner}>
          <div style={{ flex: 1 }}>
            {issues.map((issue, i) => (
              <div key={i}>• {issue}</div>
            ))}
          </div>
          <button onClick={() => setBannerDismissed(true)} style={styles.dismiss}>
            Dismiss
          </button>
        </div>
      )}
      <main style={styles.main}>
        {tab === "ask" && <AskTab />}
        {tab === "sources" && <SourcesTab />}
        {tab === "entities" && <EntitiesTab />}
        {tab === "review" && <ReviewTab />}
      </main>
    </div>
  );
}

const styles: Record<string, React.CSSProperties> = {
  wrap: { minHeight: "100vh", background: "#0f1115", color: "#e6e8eb", fontFamily: "system-ui, sans-serif" },
  header: {
    display: "flex",
    alignItems: "center",
    gap: 24,
    padding: "0.75rem 1.5rem",
    borderBottom: "1px solid #1f242c",
    background: "#14171d",
  },
  title: { fontSize: 18, margin: 0 },
  nav: { display: "flex", gap: 4 },
  tab: {
    background: "none",
    border: "none",
    color: "#9ca3af",
    padding: "0.4rem 0.8rem",
    borderRadius: 6,
    cursor: "pointer",
    fontSize: 14,
  },
  tabActive: { background: "#1e2430", color: "#e6e8eb" },
  banner: {
    display: "flex",
    alignItems: "flex-start",
    gap: 12,
    background: "#3a1d1d",
    color: "#fca5a5",
    padding: "0.6rem 1.5rem",
    fontSize: 13,
    borderBottom: "1px solid #4c2626",
  },
  dismiss: { background: "none", border: "1px solid #6b3030", color: "#fca5a5", borderRadius: 6, padding: "0.2rem 0.6rem", cursor: "pointer" },
  main: { padding: "1.5rem", maxWidth: 1000, margin: "0 auto" },
};
