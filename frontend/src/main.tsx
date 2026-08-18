import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import App from "./App";
import { ErrorBoundary } from "./ErrorBoundary";
import { ProjectsProvider } from "./ProjectsContext";

const queryClient = new QueryClient({
  defaultOptions: { queries: { staleTime: 0, retry: 1, refetchOnWindowFocus: false } },
});

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <ProjectsProvider>
        <ErrorBoundary label="app">
          <App />
        </ErrorBoundary>
      </ProjectsProvider>
    </QueryClientProvider>
  </StrictMode>,
);
