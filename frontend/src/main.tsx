import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import "./theme.css";
import App from "./App";
import { ErrorBoundary } from "./ErrorBoundary";
import { ProjectsProvider } from "./ProjectsContext";
import { StartupGate } from "./StartupGate";

const queryClient = new QueryClient({
  defaultOptions: { queries: { staleTime: 0, retry: 1, refetchOnWindowFocus: false } },
});

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    {/* R41.7: nothing below may call the API before the database is ready. */}
    <StartupGate>
      <QueryClientProvider client={queryClient}>
        <ProjectsProvider>
          <ErrorBoundary label="app">
            <App />
          </ErrorBoundary>
        </ProjectsProvider>
      </QueryClientProvider>
    </StartupGate>
  </StrictMode>,
);
