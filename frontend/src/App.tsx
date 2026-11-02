import { useEffect, useState } from "react";
import { useRoute, type Tab } from "./useRoute";
import { useQuery } from "@tanstack/react-query";
import { api } from "./api";
import { ThemeSwitch } from "./ThemeSwitch";
import { ErrorBoundary } from "./ErrorBoundary";
import { askNotice, modelIssues } from "./modelHealth";
import { useProjects } from "./useProjects";
import { theme, radius } from "./theme";
import { OnboardingWizard } from "./OnboardingWizard";
import { UpdateBanner } from "./Updates";
import { AskTab } from "./tabs/AskTab";
import { RecordsTab } from "./tabs/RecordsTab";
import { SettingsTab } from "./tabs/SettingsTab";
import { SourcesTab } from "./tabs/SourcesTab";
import { SystemTab } from "./tabs/SystemTab";
import { isActiveJob } from "./jobControlsText";
import type { Job } from "./types";

function badgeText(jobs: Job[]): string {
  const running = jobs.filter((j) => j.status === "running").length;
  const queued = jobs.length - running;
  return queued > 0 ? `${running} running · ${queued} queued` : `${running} running`;
}

// Primary = everyday flow; secondary = configure & inspect (airy grouped nav)
// R45.25: Entities became Records (decisions, actions, glossary); the review
// queue is gone from the UI, /entities* and /review/* stay for agents.
const PRIMARY_TABS: { id: Tab; label: string }[] = [
  { id: "ask", label: "Ask" },
  { id: "sources", label: "Sources" },
  { id: "records", label: "Records" },
];
const SECONDARY_TABS: { id: Tab; label: string }[] = [
  { id: "settings", label: "Settings" },
  { id: "system", label: "System" },
];

export default function App() {
  // Wave 1: the tab is in the URL hash (reload, back, deep links).
  const [tab, setTab] = useRoute();
  const [askNoticeHidden, setAskNoticeHidden] = useState(() => readSession("ask-notice-dismissed") === "1");
  const [dismissedSig, setDismissedSig] = useState<string | null>(() => localStorage.getItem("banner-dismissed"));
  const [selectedProject, setSelectedProject] = useState<number | null>(null);
  const [showWizard, setShowWizard] = useState(false);
  // R30.4: bumping this scrolls the System tab to its job list
  const [focusJobs, setFocusJobs] = useState(0);
  // B37: projects come from the shared context (no duplicate fetch here)
  const { projects } = useProjects();

  // B37: health + running-jobs polling via TanStack Query (replaces setInterval)
  const { data: health, isError: healthError } = useQuery({
    queryKey: ["health"],
    queryFn: () => api.health(),
    refetchInterval: 30_000,
    refetchIntervalInBackground: true,
  });
  // R30.4: active = running + queued, so the badge counts every job a Stop can target.
  const { data: runningJobs } = useQuery({
    queryKey: ["running-jobs"],
    queryFn: async () => (await api.listJobs(undefined, 50)).filter(isActiveJob),
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
  const others = failingForBanner.map((s) => `Sync failing: ${s.name} (${s.count}×): ${s.error ?? "unknown error"}`);
  const othersSig = JSON.stringify(others);
  const visibleOthers = dismissedSig === othersSig ? [] : others;
  // Wave 1 (one notice rule): model issues are a header dot plus one
  // dismissible line on Ask — no longer a red banner on every tab. The
  // banner is left for real failures (a source whose sync keeps failing).
  const bannerVisible = visibleOthers.length > 0;
  const notice = askNotice(health);
  const status = statusDot(online, model, others.length);

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
            onClick={() => {
              setTab("system");
              setFocusJobs((n) => n + 1);
            }}
            title={[
              ...runningJobs.map((j) => `#${j.id} ${j.kind} ${j.status}${j.total > 0 ? ` ${j.processed}/${j.total}` : ""}`),
              "Open the job list to stop a job",
            ].join("\n")}
            style={styles.jobsBadge}
          >
            <span style={styles.spinner} />
            {badgeText(runningJobs)}
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
        {status && (
          <button
            onClick={() => setTab(status.target)}
            title={status.title}
            aria-label={status.title}
            style={{ ...styles.statusDot, color: status.color }}
          >
            <span aria-hidden="true" style={{ ...styles.dot, background: status.color }} />
            {status.label}
          </button>
        )}
        <ThemeSwitch />
      </header>
      <UpdateBanner onOpen={() => setTab("system")} />
      {bannerVisible && (
        <div style={styles.banner}>
          <div style={{ flex: 1 }}>
            {visibleOthers.map((issue, i) => (
              <div key={`o${i}`}>• {issue}</div>
            ))}
          </div>
          <div style={{ display: "flex", gap: 6, flexWrap: "wrap", alignItems: "flex-start", flexShrink: 0 }}>
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
        {tab === "ask" && notice && !askNoticeHidden && (
          <p style={styles.askNotice}>
            {notice}{" "}
            <button onClick={() => setShowWizard(true)} style={styles.noticeLink}>
              Set up
            </button>
            <button
              onClick={() => {
                writeSession("ask-notice-dismissed", "1");
                setAskNoticeHidden(true);
              }}
              style={styles.noticeClose}
              aria-label="Dismiss for this session"
              title="Dismiss for this session"
            >
              ×
            </button>
          </p>
        )}
        {/* Wave 1 (UX audit #2): Ask stays mounted so switching tabs keeps
            the conversation; the other tabs are server views and remount. */}
        <div hidden={tab !== "ask"}>
          <ErrorBoundary label="Ask">
            <AskTab projectId={effectiveProject ?? undefined} onGoSources={() => setTab("sources")} />
          </ErrorBoundary>
        </div>
        <ErrorBoundary label="Sources">
          {tab === "sources" && <SourcesTab />}
        </ErrorBoundary>
        <ErrorBoundary label="Records">
          {tab === "records" && <RecordsTab />}
        </ErrorBoundary>
        <ErrorBoundary label="Settings">
          {tab === "settings" && <SettingsTab />}
        </ErrorBoundary>
        <ErrorBoundary label="System">
          {tab === "system" && <SystemTab onConfigure={() => setTab("settings")} focusJobs={focusJobs} />}
        </ErrorBoundary>
      </main>
    </div>
  );
}

function readSession(key: string): string | null {
  try {
    return sessionStorage.getItem(key);
  } catch {
    return null;
  }
}

function writeSession(key: string, value: string) {
  try {
    sessionStorage.setItem(key, value);
  } catch {
    /* private mode: the notice just comes back on reload */
  }
}

type Status = { color: string; label: string; title: string; target: Tab };

/** One header dot for every health state; null when all is well. */
function statusDot(online: boolean | null, model: string[], failing: number): Status | null {
  if (online === false) return { color: theme.red, label: "Offline", title: "The AnchorCore server is not answering", target: "system" };
  if (failing > 0) {
    const title = `${failing} source${failing === 1 ? "" : "s"} failing to sync`;
    return { color: theme.red, label: "Sync failing", title, target: "sources" };
  }
  if (model.length > 0) return { color: theme.amber, label: "Models", title: model.join("\n"), target: "settings" };
  return null;
}

const styles: Record<string, React.CSSProperties> = {
  statusDot: {
    display: "flex",
    alignItems: "center",
    gap: 6,
    background: "none",
    border: "none",
    padding: 0,
    fontSize: 12,
    cursor: "pointer",
  },
  dot: { width: 8, height: 8, borderRadius: "50%", display: "inline-block", flexShrink: 0 },
  askNotice: {
    display: "flex",
    alignItems: "baseline",
    gap: 8,
    flexWrap: "wrap",
    margin: "0 0 20px",
    fontSize: 13,
    color: theme.textMuted,
  },
  noticeLink: { background: "none", border: "none", padding: 0, color: theme.text, textDecoration: "underline", cursor: "pointer", fontSize: 13 },
  noticeClose: { marginLeft: "auto", background: "none", border: "none", padding: "0 4px", color: theme.textDim, cursor: "pointer", fontSize: 16, lineHeight: 1 },
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
