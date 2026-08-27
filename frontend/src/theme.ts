/** Shared visual tokens (B37). Replaces per-file inline `styles` color
 * literals so the palette stays consistent and changeable in one place.
 * Palette: C3 Stone & Sage — vault/archive, matte, non-AI (paper #F2F0EB, sage #4A5A52). */
export const theme = {
  bg: "#121416",
  bgElevated: "#1A1E20",
  bgCard: "#23282B",
  bgHover: "#2c3235",
  border: "#343a3e",
  borderSoft: "#2a2f33",
  text: "#E8E6E1",
  textMuted: "#9aa0a8",
  textDim: "#7a828c",
  accent: "#4A5A52",
  accentAlt: "#8FA99E",
  purple: "#6E7D75",
  green: "#7a9a8a",
  amber: "#9A8B7A",
  red: "#c98a7a",
  redBg: "#2B1E1D",
  redBorder: "#3d2a28",
  redText: "#e8c4bc",
  blue: "#7E9AB0",
  blueBg: "#1d2a33",
  buttonBg: "#2c3235",
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
