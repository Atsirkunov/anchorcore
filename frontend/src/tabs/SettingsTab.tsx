import { BackupCard } from "../BackupCard";
import { ThemeSwitch } from "../ThemeSwitch";
import { radius, theme } from "../theme";
import type { SettingsSection } from "../useRoute";
import { GlossaryPanel } from "./GlossaryPanel";
import { SettingsForm } from "./settings/SettingsForm";
import { StatusSection } from "./settings/StatusSection";

/** R49.9: Settings is a left rail, one section per page (each has its own
 *  `#/settings/<section>` route). System folded in as Status & logs; the
 *  glossary moved here from Records; the theme switch from the header. */
const RAIL: { id: SettingsSection; label: string }[] = [
  { id: "answers", label: "Answers" },
  { id: "search", label: "Search" },
  { id: "sharing", label: "Sensitive sharing" },
  { id: "glossary", label: "Glossary" },
  { id: "backup", label: "Backup & restore" },
  { id: "updates", label: "Updates" },
  { id: "status", label: "Status & logs" },
  { id: "appearance", label: "Appearance" },
  { id: "advanced", label: "Advanced" },
];

export function SettingsTab({
  section,
  onSection,
  onTour,
  focusJobs,
}: {
  section: SettingsSection;
  onSection: (s: SettingsSection) => void;
  onTour: () => void;
  focusJobs?: number;
}) {
  const title = RAIL.find((r) => r.id === section)?.label ?? "Settings";
  return (
    <div style={styles.layout}>
      <nav aria-label="Settings sections" style={styles.rail}>
        {RAIL.map((r) => (
          <a
            key={r.id}
            href={`#/settings/${r.id}`}
            aria-current={section === r.id ? "page" : undefined}
            onClick={(e) => {
              e.preventDefault();
              onSection(r.id);
            }}
            style={{ ...styles.railItem, ...(section === r.id ? styles.railActive : {}) }}
          >
            {r.label}
          </a>
        ))}
      </nav>
      <section style={{ flex: "1 1 520px", minWidth: 0 }}>
        <h2 style={{ marginTop: 0 }}>{title}</h2>
        <SectionBody section={section} onSection={onSection} onTour={onTour} focusJobs={focusJobs} />
      </section>
    </div>
  );
}

function SectionBody({ section, onSection, onTour, focusJobs }: {
  section: SettingsSection;
  onSection: (s: SettingsSection) => void;
  onTour: () => void;
  focusJobs?: number;
}) {
  switch (section) {
    case "glossary":
      return <GlossaryPanel defaultOpen />;
    case "backup":
      return <BackupCard />;
    case "status":
      return <StatusSection onConfigure={() => onSection("answers")} focusJobs={focusJobs} />;
    case "appearance":
      return (
        <div>
          <p style={{ margin: "0 0 10px", fontSize: 15, color: theme.textMuted }}>
            Auto follows your computer's light or dark setting.
          </p>
          <ThemeSwitch />
        </div>
      );
    case "advanced":
      return (
        <>
          <SettingsForm section="advanced" />
          <div style={{ marginTop: 16 }}>
            <button onClick={onTour} style={styles.button}>
              Show the welcome tour
            </button>
          </div>
        </>
      );
    default:
      return <SettingsForm section={section} />;
  }
}

const styles: Record<string, React.CSSProperties> = {
  // flex-wrap: on a phone the rail sits above the section instead of squeezing it
  layout: { display: "flex", flexWrap: "wrap", gap: 48, alignItems: "flex-start" },
  rail: { flex: "0 0 210px", display: "flex", flexDirection: "column", gap: 2 },
  railItem: { padding: "0.55rem 0.8rem", borderRadius: radius, color: theme.textMuted, textDecoration: "none", fontSize: 16 },
  railActive: { background: theme.bgHover, color: theme.text, fontWeight: 600 },
  button: { padding: "0.5rem 1rem", borderRadius: radius, border: `1px solid ${theme.border}`, background: theme.buttonBg, color: theme.text, cursor: "pointer", fontSize: 15 },
};
