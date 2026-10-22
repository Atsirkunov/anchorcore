import { useEffect, useState } from "react";
import { api } from "./api";
import { commonStyles, theme, radius } from "./theme";
import type { UpdateStatus } from "./types";

/**
 * R29.1 "Updates" card (System tab).
 *
 * Trust contract: the Check button is the ONLY thing in this component that can
 * make a network request. Loading the card / the banner performs a plain
 * `GET /system/update`, which the server answers from its cache — a plain GET
 * must never contact github.com (pinned by a Rust test).
 */
export function UpdateCard({ onGoSettings }: { onGoSettings?: () => void } = {}) {
  const [status, setStatus] = useState<UpdateStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api.updateStatus().then(setStatus).catch(() => {});
  }, []);

  const check = async () => {
    setBusy(true);
    setError(null);
    try {
      setStatus(await api.updateStatus(true));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const checkedAt = status?.checked_at ? new Date(status.checked_at).toLocaleString() : null;
  return (
    <div style={{ ...commonStyles.card, marginTop: 16 }}>
      <div style={{ display: "flex", justifyContent: "space-between", gap: 8, flexWrap: "wrap" }}>
        <div style={{ fontWeight: 600 }}>Updates</div>
        <div style={{ fontSize: 12, color: theme.textMuted }}>current {status?.current ?? "…"}</div>
      </div>
      {status?.update_available && status.latest && (
        <div style={{ fontSize: 13, color: theme.accentAlt, marginTop: 6 }}>
          AnchorCore {status.latest} is available
          {status.published_at ? ` (released ${new Date(status.published_at).toLocaleDateString()})` : ""}.
        </div>
      )}
      {status?.checked_at && !status.update_available && !status.error && (
        <div style={{ fontSize: 13, color: theme.green, marginTop: 6 }}>You're on the newest release.</div>
      )}
      {status?.error && <div style={{ fontSize: 13, color: theme.red, marginTop: 6 }}>{status.error}</div>}
      {error && <div style={{ fontSize: 13, color: theme.red, marginTop: 6 }}>{error}</div>}
      <div style={{ display: "flex", gap: 8, marginTop: 10, flexWrap: "wrap", alignItems: "center" }}>
        <button style={commonStyles.button} onClick={check} disabled={busy}>
          {busy ? "Checking…" : "Check for updates"}
        </button>
        {status?.update_available && status.asset_url && (
          <a href={status.asset_url} style={{ ...commonStyles.button, textDecoration: "none" }}>
            Download
          </a>
        )}
        {status?.release_url && (
          <a
            href={status.release_url}
            target="_blank"
            rel="noreferrer"
            style={{ fontSize: 12, color: theme.textMuted }}
          >
            Release notes
          </a>
        )}
      </div>
      <div style={{ fontSize: 11, color: theme.textMuted, marginTop: 8, lineHeight: 1.5 }}>
        Contacts github.com only when you click — a plain fetch of the public release info; nothing about you or
        your knowledge is sent. Updating replaces the app binary; your knowledge stays in <code>~/.anchorcore</code>.
        On Windows close the app before replacing it, or let your agent run the <code>anchorcore-setup</code> skill.
      </div>
      <div style={{ fontSize: 11, color: theme.textMuted, marginTop: 4 }}>
        Startup checks: <b>{status?.check_enabled ? "on" : "off"}</b>
        {!status?.check_enabled && onGoSettings && (
          <>
            {" — "}
            <button onClick={onGoSettings} style={styles.linkButton}>
              enable in Settings
            </button>
          </>
        )}
        {checkedAt && <> · last checked {checkedAt}</>}
      </div>
    </div>
  );
}

/**
 * Notification banner — only ever shows a check that already happened (the
 * opt-in startup check or a manual one); it never triggers a request itself.
 * Dismissal is per version (`localStorage`), so a newer release re-appears.
 */
export function UpdateBanner({ onOpen }: { onOpen: () => void }) {
  const [status, setStatus] = useState<UpdateStatus | null>(null);
  const [dismissed, setDismissed] = useState<string | null>(() => localStorage.getItem("update-dismissed"));

  useEffect(() => {
    // Localhost-only polling until the (opt-in) startup check has a result.
    let cancelled = false;
    const load = () => {
      api
        .updateStatus()
        .then((s) => {
          if (cancelled) return;
          setStatus(s);
          if (s.checked_at) window.clearInterval(timer);
        })
        .catch(() => {});
    };
    const timer = window.setInterval(load, 30_000);
    load();
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, []);

  const latest = status?.latest;
  if (!status?.update_available || !latest || dismissed === latest) return null;
  return (
    <div style={styles.banner}>
      <div style={{ flex: 1 }}>
        AnchorCore {latest} is available — you're on {status.current}.
      </div>
      <div style={{ display: "flex", gap: 6, flexShrink: 0 }}>
        <button onClick={onOpen} style={styles.bannerButton}>
          View
        </button>
        <button
          onClick={() => {
            localStorage.setItem("update-dismissed", latest);
            setDismissed(latest);
          }}
          style={styles.bannerButton}
        >
          Dismiss
        </button>
      </div>
    </div>
  );
}

const styles: Record<string, React.CSSProperties> = {
  linkButton: {
    background: "none",
    border: "none",
    color: theme.accentAlt,
    cursor: "pointer",
    padding: 0,
    fontSize: 11,
    textDecoration: "underline",
  },
  banner: {
    display: "flex",
    alignItems: "flex-start",
    gap: 12,
    background: theme.blueBg,
    color: theme.text,
    padding: "0.6rem 1.5rem",
    fontSize: 13,
    borderBottom: `1px solid ${theme.blue}`,
  },
  bannerButton: {
    background: "none",
    border: `1px solid ${theme.blue}`,
    color: theme.text,
    borderRadius: radius,
    padding: "0.2rem 0.6rem",
    cursor: "pointer",
  },
};
