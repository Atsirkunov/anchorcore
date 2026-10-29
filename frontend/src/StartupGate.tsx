import { useEffect, useState, type ReactNode } from "react";
import { commonStyles, radius, theme } from "./theme";
import { elapsedText, probeStartup, progressText, stepText, type GateState, type StartupStatus } from "./startupText";

const POLL_MS = 1000;

/** Renders `children` once the database is ready; until then a full-page
 * upgrade screen (polling `/health`), or the error screen if boot failed. */
export function StartupGate({ children, pollMs = POLL_MS }: { children: ReactNode; pollMs?: number }) {
  const [state, setState] = useState<GateState>({ kind: "checking" });
  useEffect(() => {
    if (state.kind === "ready" || state.kind === "failed") return;
    let cancelled = false;
    const delay = state.kind === "checking" ? 0 : pollMs;
    const t = setTimeout(() => {
      probeStartup().then((next) => {
        if (!cancelled) setState(next);
      });
    }, delay);
    return () => {
      cancelled = true;
      clearTimeout(t);
    };
  }, [state, pollMs]);

  if (state.kind === "ready") return <>{children}</>;
  if (state.kind === "failed") return <StartupError status={state.status} />;
  if (state.kind === "starting") return <UpgradeScreen status={state.status} />;
  // The first probe takes milliseconds on a normal start: no flash.
  if (state.kind === "checking") return null;
  return (
    <Page>
      <h1 style={titleStyle}>Waiting for AnchorCore…</h1>
    </Page>
  );
}

function Page({ children }: { children: ReactNode }) {
  return (
    <div
      style={{
        minHeight: "100vh",
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        background: theme.bg,
        color: theme.text,
        fontFamily: theme.sans,
        padding: "1rem",
      }}
    >
      <div style={{ ...commonStyles.card, maxWidth: 560, width: "100%", padding: "1.5rem 1.75rem" }}>{children}</div>
    </div>
  );
}

const titleStyle = { fontFamily: theme.serif, fontSize: "1.4rem", margin: "0 0 0.75rem" } as const;

export function UpgradeScreen({ status }: { status: StartupStatus }) {
  const p = status.progress;
  const pct = p && p.total > 0 ? Math.min(100, Math.round((p.done / p.total) * 100)) : null;
  const progress = progressText(p);
  const elapsed = elapsedText(status.detail_since);
  return (
    <Page>
      <h1 style={titleStyle}>{status.upgrading ? "Upgrading your library" : "Starting AnchorCore"}</h1>
      <p role="status" style={{ margin: "0 0 0.75rem" }}>
        {stepText(status)}
      </p>
      {pct !== null && (
        <div
          role="progressbar"
          aria-valuenow={pct}
          aria-valuemin={0}
          aria-valuemax={100}
          style={{ height: 6, background: theme.mutedBg, borderRadius: radius, marginBottom: "0.5rem" }}
        >
          <div style={{ width: `${pct}%`, height: "100%", background: theme.accent, borderRadius: radius }} />
        </div>
      )}
      {progress && <p style={{ margin: "0 0 0.75rem", color: theme.textMuted, fontSize: "0.9rem" }}>{progress}</p>}
      {elapsed && <p style={{ margin: "0 0 0.75rem", color: theme.textMuted, fontSize: "0.9rem" }}>{elapsed}</p>}
      {status.upgrading && (
        <p style={{ margin: 0, color: theme.textMuted, fontSize: "0.9rem" }}>
          This can take several minutes on large libraries. Don't close AnchorCore — this page continues by itself
          when the upgrade is done.
        </p>
      )}
    </Page>
  );
}

export function StartupError({ status }: { status: StartupStatus }) {
  const [copied, setCopied] = useState(false);
  const copy = () => {
    navigator.clipboard
      ?.writeText(status.support ?? "")
      .then(() => setCopied(true))
      .catch(() => {});
  };
  return (
    <Page>
      <h1 style={titleStyle}>AnchorCore couldn't upgrade your library</h1>
      <div
        role="alert"
        style={{
          background: theme.redBg,
          border: `1px solid ${theme.redBorder}`,
          color: theme.redText,
          borderRadius: radius,
          padding: "0.6rem 0.8rem",
          marginBottom: "0.9rem",
        }}
      >
        {status.error}
      </div>
      <h2 style={{ fontSize: "1rem", margin: "0 0 0.35rem" }}>Your data</h2>
      <p style={{ margin: "0 0 0.9rem" }}>{status.data_state}</p>
      <h2 style={{ fontSize: "1rem", margin: "0 0 0.35rem" }}>What to do</h2>
      <p style={{ margin: "0 0 0.5rem" }}>
        Restart AnchorCore to try again. If it fails again, send this to support (it holds no document content):
      </p>
      <pre
        style={{
          fontFamily: theme.mono,
          fontSize: "0.8rem",
          whiteSpace: "pre-wrap",
          background: theme.bgElevated,
          border: `1px solid ${theme.borderSoft}`,
          borderRadius: radius,
          padding: "0.6rem",
          margin: "0 0 0.5rem",
        }}
      >
        {status.support}
      </pre>
      <button type="button" style={commonStyles.button} onClick={copy}>
        {copied ? "Copied" : "Copy for support"}
      </button>
      {status.log_file && (
        <p style={{ margin: "0.9rem 0 0", color: theme.textMuted, fontSize: "0.9rem" }}>
          Full log: <code style={{ fontFamily: theme.mono }}>{status.log_file}</code>
        </p>
      )}
    </Page>
  );
}
