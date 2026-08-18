import { Component, type ErrorInfo, type ReactNode } from "react";
import { api } from "./api";
import { theme } from "./theme";

type Props = {
  children: ReactNode;
  /** Short label shown in the boundary, e.g. the tab name. */
  label?: string;
};

type State = {
  error: Error | null;
};

/** B36: top-level + per-tab error boundary so a render throw shows a recoverable
 * card instead of a white screen (the packaged `console=False` app has no dev
 * console to surface the stack). */
export class ErrorBoundary extends Component<Props, State> {
  state: State = { error: null };

  static getDerivedStateFromError(error: Error): State {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo): void {
    console.error("AnchorCore UI error:", error, info.componentStack);
  }

  private reset = () => this.setState({ error: null });

  render() {
    if (!this.state.error) return this.props.children;
    const { error } = this.state;
    return (
      <div style={styles.card}>
        <div style={styles.title}>Something went wrong{this.props.label ? ` — ${this.props.label}` : ""}</div>
        <pre style={styles.detail}>{error.message}</pre>
        <div style={{ display: "flex", gap: 8 }}>
          <button onClick={this.reset} style={styles.button}>
            Reload tab
          </button>
          <button
            onClick={() => navigator.clipboard?.writeText(`${error.message}\n\n${error.stack ?? ""}`).catch(() => {})}
            style={styles.button}
          >
            Copy error
          </button>
          <a href={api.logDownloadUrl("anchorcore.log")} download style={styles.link}>
            Download log
          </a>
        </div>
      </div>
    );
  }
}

const styles: Record<string, React.CSSProperties> = {
  card: { background: theme.bgCard, border: `1px solid ${theme.redBorder}`, borderRadius: 8, padding: "1rem 1.25rem", color: theme.redText },
  title: { fontWeight: 600, marginBottom: 8 },
  detail: { whiteSpace: "pre-wrap", fontSize: 12, color: theme.red, background: theme.bgElevated, padding: "0.5rem 0.75rem", borderRadius: 6, margin: "0 0 12px" },
  button: { padding: "0.4rem 0.9rem", borderRadius: 6, border: `1px solid ${theme.redBorder}`, background: theme.redBg, color: theme.redText, cursor: "pointer", fontSize: 13 },
  link: { fontSize: 13, color: theme.accentAlt, textDecoration: "none", alignSelf: "center" },
};
