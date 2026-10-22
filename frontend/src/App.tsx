import { useEffect, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { api } from "./api";
import { ThemeSwitch } from "./ThemeSwitch";
import { ErrorBoundary } from "./ErrorBoundary";
import { modelIssues } from "./modelHealth";
import { useProjects } from "./useProjects";
import { theme, radius } from "./theme";
import { OnboardingWizard } from "./OnboardingWizard";
import { UpdateBanner } from "./Updates";
import { AskTab } from "./tabs/AskTab";
import { EntitiesTab } from "./tabs/EntitiesTab";
import { SettingsTab } from "./tabs/SettingsTab";
import { SourcesTab } from "./tabs/SourcesTab";
import { SystemTab } from "./tabs/SystemTab";

type Tab = "ask" | "sources" | "entities" | "settings" | "system";

// Primary = everyday flow; secondary = configure & inspect (airy grouped nav)
// R31.2: the Review tab folded into Entities ("Needs a look" + Possible doubles);
// /review/* stays for agents.
const PRIMARY_TABS: { id: Tab; label: string }[] = [
  { id: "ask", label: "Ask" },
  { id: "sources", label: "Sources" },
  { id: "entities", label: "Entities" },
];
const SECONDARY_TABS: { id: Tab; label: string }[] = [
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

  const model = health ? modelIssues(health) : [];
  const failingForBanner = health
    ? health.components.failing_sources.filter(
        (s) => !runningJobs?.some((j) => j.source_id === s.id && (j.status === "running" || j.status === "pending")),
      )
    : [];
  const others = failingForBanner.map((s) => `Sync failing: ${s.name} (${s.count}×) — ${s.error ?? "unknown error"}`);
  const othersSig = JSON.stringify(others);
  const visibleOthers = dismissedSig === othersSig ? [] : others;
  const bannerVisible = model.length > 0 || visibleOthers.length > 0;

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
        <h1 style={styles.title}>
          {/* brand mark: same shape as favicon.svg / the website, coloured by tokens */}
          <svg width="20" height="20" viewBox="0 0 64 64" aria-hidden="true" style={{ flexShrink: 0 }}>
            <rect x="2" y="2" width="60" height="60" rx="14" fill="none" stroke={theme.border} strokeWidth="4" />
            <rect x="16" y="12" width="10" height="40" rx="3" fill={theme.cite} />
            <rect x="32" y="17" width="16" height="6" rx="3" fill={theme.textDim} />
            <rect x="32" y="29" width="12" height="6" rx="3" fill={theme.textDim} />
            <rect x="32" y="41" width="16" height="6" rx="3" fill={theme.textDim} />
          </svg>
          AnchorCore
        </h1>
        <nav style={styles.nav}>
          {PRIMARY_TABS.map((t) => (
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
          <span style={styles.navDivider} aria-hidden="true" />
          {SECONDARY_TABS.map((t) => (
            <button
              key={t.id}
              onClick={() => setTab(t.id)}
              title="Configure & inspect"
              style={{
                ...styles.tab,
                ...styles.tabSecondary,
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
          title="Scope questions to a project"
        >
          <option value="">All sources</option>
          {projects.map((p) => (
            <option key={p.id} value={p.id}>
              {p.name}
              {p.is_default ? " ★" : ""}
            </option>
          ))}
        </select>
        <span style={{ color: online === false ? theme.red : online ? theme.green : theme.textDim, fontSize: 12, fontFamily: theme.mono }}>
          {online === false ? "API offline" : online ? "API online" : "checking…"}
        </span>
        <ThemeSwitch />
      </header>
      <UpdateBanner onOpen={() => setTab("system")} />
      {bannerVisible && (
        <div style={styles.banner}>
          <div style={{ flex: 1 }}>
            {model.map((issue, i) => (
              <div key={`m${i}`}>• {issue}</div>
            ))}
            {visibleOthers.map((issue, i) => (
              <div key={`o${i}`}>• {issue}</div>
            ))}
          </div>
          <div style={{ display: "flex", gap: 6, flexWrap: "wrap", alignItems: "flex-start", flexShrink: 0 }}>
            {model.length > 0 && (
              <>
                <button onClick={() => setShowWizard(true)} style={styles.dismiss}>
                  Set up models
                </button>
                <button onClick={() => setTab("settings")} style={styles.dismiss}>
                  Settings
                </button>
              </>
            )}
            {visibleOthers.length > 0 && (
              <button
                onClick={() => {
                  localStorage.setItem("banner-dismissed", othersSig);
                  setDismissedSig(othersSig);
                }}
                style={styles.dismiss}
              >
                Dismiss
              </button>
            )}
          </div>
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
  wrap: { minHeight: "100vh", background: theme.bg, color: theme.text, fontFamily: theme.sans },
  header: {
    display: "flex",
    alignItems: "center",
    gap: 20,
    rowGap: 12,
    flexWrap: "wrap",
    padding: "0 2rem",
    borderBottom: `1px solid ${theme.text}`,
    background: theme.bg,
  },
  title: { display: "flex", alignItems: "center", gap: 8, fontFamily: theme.sans, fontSize: 17, fontWeight: 600, letterSpacing: "-0.01em", margin: "0 8px 0 0" },
  nav: { display: "flex", gap: 4, alignItems: "center", flexWrap: "wrap", alignSelf: "stretch" },
  navDivider: { width: 1, height: 18, background: theme.border, margin: "0 8px" },
  tabSecondary: { color: theme.textDim, fontSize: 13 },
  tab: {
    background: "none",
    border: "none",
    color: theme.textMuted,
    padding: "1.1rem 0.7rem 1rem",
    borderRadius: 0,
    borderBottom: "2px solid transparent",
    marginBottom: -1,
    cursor: "pointer",
    fontSize: 14,
  },
  tabActive: { color: theme.text, borderBottom: `2px solid ${theme.text}`, fontWeight: 600 },
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
  dismiss: { background: "none", border: `1px solid ${theme.redBorder}`, color: theme.redText, borderRadius: radius, padding: "0.2rem 0.6rem", cursor: "pointer" },
  main: { padding: "2.5rem 2rem 4rem", maxWidth: 880, margin: "0 auto" },
  jobsBadge: {
    display: "flex",
    alignItems: "center",
    gap: 6,
    background: theme.bgCard,
    border: `1px solid ${theme.accentAlt}`,

    color: theme.accentAlt,
    padding: "0.25rem 0.6rem",
    borderRadius: radius,
    fontSize: 12,
    cursor: "pointer",
  },
  projectPicker: {
    background: theme.bgCard,
    border: `1px solid ${theme.border}`,
    color: theme.text,
    borderRadius: radius,
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
