import { createContext, useContext } from "react";
import type { Project } from "./types";

export type ProjectsContextValue = {
  projects: Project[];
  refreshProjects: () => void;
};

export const ProjectsContext = createContext<ProjectsContextValue>({
  projects: [],
  refreshProjects: () => {},
});

/** B37: single source of truth for the projects list — kept separate from
 * ProjectsProvider (ProjectsContext.tsx) so Fast Refresh works on the component file. */
export function useProjects(): ProjectsContextValue {
  return useContext(ProjectsContext);
}
