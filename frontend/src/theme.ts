/** Shared visual tokens (B37). Replaces per-file inline `styles` color
 * literals so the palette stays consistent and changeable in one place. */
export const theme = {
  bg: "#0f1115",
  bgElevated: "#14171d",
  bgCard: "#171a21",
  bgHover: "#1e2430",
  border: "#2d333b",
  borderSoft: "#1f242c",
  text: "#e6e8eb",
  textMuted: "#9ca3af",
  textDim: "#6b7280",
  accent: "#6366f1",
  accentAlt: "#7dd3fc",
  purple: "#8b5cf6",
  green: "#4ade80",
  amber: "#fbbf24",
  red: "#f87171",
  redBg: "#3a1d1d",
  redBorder: "#6b3030",
  redText: "#fca5a5",
  blue: "#38bdf8",
  blueBg: "#1e3a5f",
  buttonBg: "#2b3240",
};

export const commonStyles = {
  input: {
    padding: "0.55rem 0.75rem",
    borderRadius: 8,
    border: `1px solid ${theme.border}`,
    background: theme.bgCard,
    color: theme.text,
  },
  button: {
    padding: "0.55rem 1rem",
    borderRadius: 8,
    border: "none",
    background: theme.buttonBg,
    color: theme.text,
    cursor: "pointer",
  },
  card: {
    background: theme.bgCard,
    border: `1px solid ${theme.border}`,
    borderRadius: 8,
    padding: "0.75rem 1rem",
  },
} as const satisfies Record<string, React.CSSProperties>;
