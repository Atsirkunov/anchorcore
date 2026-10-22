/** Shared visual tokens (B37). Values are CSS variables defined in theme.css
 * (light by default, dark via prefers-color-scheme), so every inline style
 * follows the OS theme without re-rendering. Palette: Archive — ink on paper,
 * oxblood for citations and links, shared with anchorcore.dev. */
export const theme = {
  bg: "var(--ac-bg)",
  bgElevated: "var(--ac-bg-elevated)",
  bgCard: "var(--ac-bg-card)",
  bgHover: "var(--ac-bg-hover)",
  border: "var(--ac-border)",
  borderSoft: "var(--ac-border-soft)",
  text: "var(--ac-text)",
  textMuted: "var(--ac-text-muted)",
  textDim: "var(--ac-text-dim)",
  // primary buttons: ink
  accent: "var(--ac-accent)",
  // links, highlights and citation marks: oxblood
  accentAlt: "var(--ac-cite)",
  cite: "var(--ac-cite)",
  citeSoft: "var(--ac-cite-soft)",
  onAccent: "var(--ac-on-accent)",
  greenBg: "var(--ac-green-bg)",
  greenText: "var(--ac-green-text)",
  mutedBg: "var(--ac-muted-bg)",
  amberBg: "var(--ac-amber-bg)",
  markText: "var(--ac-mark-text)",
  purple: "var(--ac-slate)",
  green: "var(--ac-green)",
  amber: "var(--ac-amber)",
  red: "var(--ac-red)",
  redBg: "var(--ac-red-bg)",
  redBorder: "var(--ac-red-border)",
  redText: "var(--ac-red-text)",
  blue: "var(--ac-blue)",
  blueBg: "var(--ac-blue-bg)",
  buttonBg: "var(--ac-button-bg)",
  overlay: "var(--ac-overlay)",
  shadow: "var(--ac-shadow)",
  sans: "var(--ac-sans)",
  serif: "var(--ac-serif)",
  mono: "var(--ac-mono)",
};

/** Corner radius for every box: square, document-like. Dots keep 50%. */
export const radius = 2;

export const commonStyles = {
  input: {
    padding: "0.55rem 0.75rem",
    borderRadius: radius,
    border: `1px solid ${theme.border}`,
    background: theme.bgCard,
    color: theme.text,
  },
  button: {
    padding: "0.55rem 1rem",
    borderRadius: radius,
    border: `1px solid ${theme.border}`,
    background: theme.buttonBg,
    color: theme.text,
    cursor: "pointer",
  },
  card: {
    background: theme.bgCard,
    border: `1px solid ${theme.border}`,
    borderRadius: radius,
    padding: "0.75rem 1rem",
  },
} as const satisfies Record<string, React.CSSProperties>;
