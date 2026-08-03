import { useEffect, useState } from "react";
import { api } from "./api";
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

  useEffect(() => {
    api
      .health()
      .then(() => setOnline(true))
      .catch(() => setOnline(false));
  }, []);

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
  main: { padding: "1.5rem", maxWidth: 1000, margin: "0 auto" },
};
