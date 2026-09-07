import { useEffect, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { api } from "./api";
import { ErrorBoundary } from "./ErrorBoundary";
import { useProjects } from "./useProjects";
import { theme } from "./theme";
import { OnboardingWizard } from "./OnboardingWizard";
import { AskTab } from "./tabs/AskTab";
import { EntitiesTab } from "./tabs/EntitiesTab";
import { PiiTab } from "./tabs/PiiTab";
import { ReviewTab } from "./tabs/ReviewTab";
import { SettingsTab } from "./tabs/SettingsTab";
import { SourcesTab } from "./tabs/SourcesTab";
import { SystemTab } from "./tabs/SystemTab";

type Tab = "ask" | "sources" | "entities" | "review" | "settings" | "system" | "pii";

const TABS: { id: Tab; label: string }[] = [
  { id: "ask", label: "Ask" },
  { id: "sources", label: "Sources" },
  { id: "entities", label: "Entities" },
  { id: "review", label: "Review" },
  { id: "pii", label: "PII" },
  { id: "settings", label: "Settings" },
  { id: "system", label: "System" },
];

export default function App() {
  const [tab, setTab] = useState<Tab>("ask");
  const [dismissedSig, setDismissedSig] = useState<string | null>(() => localStorage.getItem("banner-dismissed"));
  const [selectedProject, setSelectedProject] = useState<number | null>(null);
  const [showWizard, setShowWizard] = useState(false);
  // B37: projects come from the shared context (no duplicate fetch here)
  const { projects } = useProjects();

  // B37: health + running-jobs polling via TanStack Query (replaces setInterval)
  const { data: health, isError: healthError } = useQuery({
    queryKey: ["health"],
    queryFn: () => api.health(),
    refetchInterval: 30_000,
    refetchIntervalInBackground: true,
  });
  const { data: runningJobs } = useQuery({
    queryKey: ["running-jobs"],
    queryFn: () => api.runningJobs(),
    refetchInterval: 5_000,
    refetchIntervalInBackground: true,
  });

  const online = healthError ? false : health ? true : null;
  // effective scope = explicit user pick, else the default project
  const effectiveProject = selectedProject ?? projects.find((p) => p.is_default)?.id ?? null;

  useEffect(() => {
    let cancelled = false;
    api
      .onboarding()
      .then((o) => {
        if (cancelled) return;
        if (o.needs_wizard && !localStorage.getItem("wizard-dismissed")) setShowWizard(true);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, []);

  const failingForBanner = health
    ? health.components.failing_sources.filter(
        (s) => !runningJobs?.some((j) => j.source_id === s.id && (j.status === "running" || j.status === "pending")),
      )
    : [];
  const issues = health
    ? [
        ...(health.components.ollama === "offline" ? ["Ollama offline — classification falls back to rules. Settings → Test connection."] : []),
        ...(health.components.answer_key === "missing" ? ["No answer model — answers show context only. Set ANCHOR_ANSWER_API_KEY or point ANCHOR_ANSWER_BASE_URL at Ollama."] : []),
        ...failingForBanner.map((s) => `Sync failing: ${s.name} (${s.count}×) — ${s.error ?? "unknown error"}`),
      ]
    : [];
  const issuesSig = JSON.stringify(issues);
  const bannerVisible = issues.length > 0 && dismissedSig !== issuesSig;

  return (
    <div style={styles.wrap}>
      {showWizard && (
        <OnboardingWizard
          onClose={() => {
            localStorage.setItem("wizard-dismissed", "1");
            setShowWizard(false);
          }}
        />
      )}
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
        {runningJobs && runningJobs.length > 0 && (
          <button
            onClick={() => setTab("sources")}
            title={runningJobs.map((j) => `#${j.id} ${j.kind}${j.total > 0 ? ` ${j.processed}/${j.total}` : ""}`).join("\n")}
            style={styles.jobsBadge}
          >
            <span style={styles.spinner} />
            {runningJobs.length} running
          </button>
        )}
        <select
          value={effectiveProject ?? ""}
          onChange={(e) => setSelectedProject(e.target.value ? Number(e.target.value) : null)}
          style={styles.projectPicker}
          title="Scope questions to a project (B15)"
        >
          <option value="">All sources</option>
          {projects.map((p) => (
            <option key={p.id} value={p.id}>
              {p.name}
              {p.is_default ? " ★" : ""}
            </option>
          ))}
        </select>
        <span style={{ color: online === false ? theme.red : online ? theme.green : theme.textDim, fontSize: 12 }}>
          {online === false ? "API offline" : online ? "API online" : "checking…"}
        </span>
      </header>
      {bannerVisible && (
        <div style={styles.banner}>
          <div style={{ flex: 1 }}>
            {issues.map((issue, i) => (
              <div key={i}>• {issue}</div>
            ))}
          </div>
          <button
            onClick={() => {
              localStorage.setItem("banner-dismissed", issuesSig);
              setDismissedSig(issuesSig);
            }}
            style={styles.dismiss}
          >
            Dismiss
          </button>
        </div>
      )}
      <main style={styles.main}>
        <ErrorBoundary label="Ask">
          {tab === "ask" && <AskTab projectId={effectiveProject ?? undefined} onGoSources={() => setTab("sources")} />}
        </ErrorBoundary>
        <ErrorBoundary label="Sources">
          {tab === "sources" && <SourcesTab />}
        </ErrorBoundary>
        <ErrorBoundary label="Entities">
          {tab === "entities" && <EntitiesTab />}
        </ErrorBoundary>
        <ErrorBoundary label="Review">
          {tab === "review" && <ReviewTab />}
        </ErrorBoundary>
        <ErrorBoundary label="PII">
          {tab === "pii" && <PiiTab />}
        </ErrorBoundary>
        <ErrorBoundary label="Settings">
          {tab === "settings" && <SettingsTab />}
        </ErrorBoundary>
        <ErrorBoundary label="System">
          {tab === "system" && <SystemTab onConfigure={() => setTab("settings")} />}
        </ErrorBoundary>
      </main>
    </div>
  );
}

const styles: Record<string, React.CSSProperties> = {
  wrap: { minHeight: "100vh", background: theme.bg, color: theme.text, fontFamily: "system-ui, sans-serif" },
  header: {
    display: "flex",
    alignItems: "center",
    gap: 24,
    padding: "0.75rem 1.5rem",
    borderBottom: `1px solid ${theme.borderSoft}`,
    background: theme.bgElevated,
  },
  title: { fontSize: 18, margin: 0 },
  nav: { display: "flex", gap: 4 },
  tab: {
    background: "none",
    border: "none",
    color: theme.textMuted,
    padding: "0.4rem 0.8rem",
    borderRadius: 6,
    cursor: "pointer",
    fontSize: 14,
  },
  tabActive: { background: theme.bgHover, color: theme.text },
  banner: {
    display: "flex",
    alignItems: "flex-start",
    gap: 12,
    background: theme.redBg,
    color: theme.redText,
    padding: "0.6rem 1.5rem",
    fontSize: 13,
    borderBottom: `1px solid ${theme.redBorder}`,
  },
  dismiss: { background: "none", border: `1px solid ${theme.redBorder}`, color: theme.redText, borderRadius: 6, padding: "0.2rem 0.6rem", cursor: "pointer" },
  main: { padding: "1.5rem", maxWidth: 1000, margin: "0 auto" },
  jobsBadge: {
    display: "flex",
    alignItems: "center",
    gap: 6,
    background: theme.bgHover,
    border: `1px solid ${theme.accentAlt}`,
    color: theme.accentAlt,
    padding: "0.25rem 0.6rem",
    borderRadius: 999,
    fontSize: 12,
    cursor: "pointer",
  },
  projectPicker: {
    background: theme.bgCard,
    border: `1px solid ${theme.border}`,
    color: theme.text,
    borderRadius: 6,
    padding: "0.3rem 0.6rem",
    fontSize: 13,
    maxWidth: 180,
  },
  spinner: {
    width: 10,
    height: 10,
    borderRadius: "50%",
    border: `2px solid ${theme.border}`,
    borderTopColor: theme.accentAlt,
    animation: "spin 0.8s linear infinite",
    flexShrink: 0,
  },
};
