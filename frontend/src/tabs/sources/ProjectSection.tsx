import { useState } from "react";
import { api } from "../../api";
import { useProjects } from "../../ProjectsContext";
import { theme, commonStyles } from "../../theme";
import type { Source } from "../../types";

/** B37: project create + per-source membership toggles (moved out of the
 * god SourcesTab). Consumes the shared ProjectsContext so there's exactly one
 * /projects fetch. */
export function ProjectSection({ sources, onChanged }: { sources: Source[]; onChanged: () => void }) {
  const { projects, refreshProjects } = useProjects();
  const [projectName, setProjectName] = useState("");
  const [error, setError] = useState<string | null>(null);

  async function createProject() {
    if (!projectName.trim()) return;
    setError(null);
    try {
      await api.createProject({ name: projectName.trim() });
      setProjectName("");
      refreshProjects();
      onChanged();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  async function toggleSourceInProject(sourceId: number, project: { id: number; source_ids: number[] }) {
    setError(null);
    try {
      const has = project.source_ids.includes(sourceId);
      const next = has
        ? project.source_ids.filter((id) => id !== sourceId)
        : [...project.source_ids, sourceId];
      await api.updateProject(project.id, { source_ids: next });
      refreshProjects();
      onChanged();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  async function setDefaultProject(project: { id: number }) {
    setError(null);
    try {
      await api.updateProject(project.id, { is_default: true });
      refreshProjects();
      onChanged();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  async function deleteProject(project: { id: number }) {
    setError(null);
    try {
      await api.deleteProject(project.id);
      refreshProjects();
      onChanged();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  return (
    <div
      style={{
        background: theme.bgElevated,
        border: `1px solid ${theme.border}`,
        borderRadius: 8,
        padding: "0.9rem 1rem",
        marginBottom: 16,
      }}
    >
      <div style={{ fontWeight: 600, marginBottom: 8 }}>
        Projects (scope questions by source group — B15)
      </div>
      <div style={{ display: "flex", gap: 8, marginBottom: 8 }}>
        <input
          style={commonStyles.input}
          placeholder="New project name"
          value={projectName}
          onChange={(e) => setProjectName(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && createProject()}
        />
        <button onClick={createProject} style={commonStyles.button}>
          Create
        </button>
      </div>
      {error && <p style={{ color: theme.red }}>{error}</p>}
      {projects.length === 0 ? (
        <div style={{ fontSize: 12, color: theme.textDim }}>
          No projects yet — create one, then tick sources to include them.
        </div>
      ) : (
        <div style={{ display: "grid", gap: 8 }}>
          {projects.map((p) => (
            <div key={p.id} style={{ border: `1px solid ${theme.border}`, borderRadius: 6, padding: "0.5rem 0.7rem" }}>
              <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between" }}>
                <span style={{ fontWeight: 600, fontSize: 13 }}>
                  {p.name} {p.is_default && <span title="default scope">★</span>}
                </span>
                <div style={{ display: "flex", gap: 6 }}>
                  {!p.is_default && (
                    <button
                      onClick={() => setDefaultProject(p)}
                      style={{ ...commonStyles.button, fontSize: 12, padding: "0.2rem 0.5rem" }}
                    >
                      Make default
                    </button>
                  )}
                  <button
                    onClick={() => deleteProject(p)}
                    style={{ ...commonStyles.button, fontSize: 12, padding: "0.2rem 0.5rem", background: theme.redBg, color: theme.redText }}
                  >
                    Delete
                  </button>
                </div>
              </div>
              <div style={{ display: "flex", flexWrap: "wrap", gap: 6, marginTop: 6 }}>
                {sources.map((s) => {
                  const inProject = p.source_ids.includes(s.id);
                  return (
                    <button
                      key={s.id}
                      onClick={() => toggleSourceInProject(s.id, p)}
                      style={{
                        ...commonStyles.button,
                        fontSize: 12,
                        padding: "0.2rem 0.6rem",
                        ...(inProject
                          ? { background: theme.blueBg, color: theme.accentAlt }
                          : { background: theme.buttonBg, color: theme.textMuted }),
                      }}
                    >
                      {inProject ? "✓ " : ""}
                      {s.name}
                    </button>
                  );
                })}
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
