import { theme, radius } from "./theme";
import { handleError } from "./skillHandle";

/** R47.4: "Do you want to use a different handle for it?" in the install confirm. */
export function SkillHandleField({ value, onChange, disabled }: { value: string; onChange: (v: string) => void; disabled?: boolean }) {
  const err = handleError(value);
  return (
    <div style={{ marginTop: 6 }}>
      <label style={{ fontSize: 12, color: theme.textMuted, display: "flex", gap: 6, alignItems: "center", flexWrap: "wrap" }}>
        Handle — what you type after / to call it:
        <span style={{ fontFamily: theme.mono }}>/</span>
        <input
          aria-label="Skill handle"
          value={value}
          disabled={disabled}
          onChange={(e) => onChange(e.target.value.toLowerCase())}
          style={{ fontFamily: theme.mono, fontSize: 12, padding: "0.2rem 0.4rem", borderRadius: radius, border: `1px solid ${err ? theme.red : theme.border}`, background: theme.bgCard, color: theme.text }}
        />
      </label>
      {err && <div style={{ fontSize: 11, color: theme.red, marginTop: 2 }}>{err}</div>}
    </div>
  );
}
