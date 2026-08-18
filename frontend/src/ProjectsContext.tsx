import { createContext, useCallback, useContext, useEffect, useState, type ReactNode } from "react";
import { api } from "./api";
import type { Project } from "./types";

type ProjectsContextValue = {
  projects: Project[];
  refreshProjects: () => void;
};

const ProjectsContext = createContext<ProjectsContextValue>({
  projects: [],
  refreshProjects: () => {},
});

/** B37: single source of truth for the projects list. App.tsx (project picker)
 * and SourcesTab (project membership) previously fetched /projects separately;
 * now both consume this context, invalidated on create/update/delete. */
export function ProjectsProvider({ children }: { children: ReactNode }) {
  const [projects, setProjects] = useState<Project[]>([]);

  const refreshProjects = useCallback(() => {
    api
      .listProjects()
      .then(setProjects)
      .catch(() => {}); // offline — keep the last known list
  }, []);

  useEffect(() => {
    refreshProjects();
  }, [refreshProjects]);

  return (
    <ProjectsContext.Provider value={{ projects, refreshProjects }}>{children}</ProjectsContext.Provider>
  );
}

export function useProjects(): ProjectsContextValue {
  return useContext(ProjectsContext);
}
