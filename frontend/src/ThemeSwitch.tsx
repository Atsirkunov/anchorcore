import { useEffect, useRef, useState } from "react";
import { theme, radius } from "./theme";
import { applyMode, followSystem, readMode, type ThemeMode } from "./themeMode";

const MODES: { id: ThemeMode; label: string; title: string }[] = [
  { id: "auto", label: "Auto", title: "Follow the system setting" },
  { id: "light", label: "Light", title: "Always light" },
  { id: "dark", label: "Dark", title: "Always dark" },
];

/** Header control: Auto / Light / Dark (see themeMode.ts). */
export function ThemeSwitch() {
  const [mode, setMode] = useState<ThemeMode>(readMode);
  const modeRef = useRef(mode);

  useEffect(() => {
    modeRef.current = mode;
  }, [mode]);
  useEffect(() => followSystem(() => modeRef.current), []);

  const choose = (m: ThemeMode) => {
    setMode(m);
    applyMode(m);
  };

  return (
    <div role="group" aria-label="Theme" style={styles.group}>
      {MODES.map((m) => (
        <button
          key={m.id}
          type="button"
          aria-pressed={mode === m.id}
          title={m.title}
          onClick={() => choose(m.id)}
          style={{ ...styles.btn, ...(mode === m.id ? styles.on : {}) }}
        >
          {m.label}
        </button>
      ))}
    </div>
  );
}

const styles: Record<string, React.CSSProperties> = {
  group: { display: "inline-flex", marginLeft: "auto", border: `1px solid ${theme.border}`, borderRadius: radius },
  btn: {
    background: "none",
    border: "none",
    padding: "0.25rem 0.55rem",
    fontFamily: theme.mono,
    fontSize: 11,
    color: theme.textDim,
    cursor: "pointer",
  },
  on: { background: theme.text, color: theme.bg },
};
