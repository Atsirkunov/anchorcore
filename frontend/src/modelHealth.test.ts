import { describe, expect, it } from "vitest";
import { modelIssues } from "./modelHealth";
import type { Health } from "./types";

function health(over: Partial<Health["components"]> = {}): Health {
  return {
    status: "ok",
    data_dir: "redacted",
    components: {
      ollama: "ok",
      answer_key: "configured",
      missing_models: [],
      answer_ready: true,
      pending_embeddings: 0,
      tasks: {},
      classifier: { windows: 0, avg_latency_ms: 0, concurrency: 4 },
      failing_sources: [],
      ...over,
    },
  };
}

describe("modelIssues", () => {
  it("is silent when models are ready", () => {
    expect(modelIssues(health())).toEqual([]);
  });

  it("is silent before health loads", () => {
    expect(modelIssues(undefined)).toEqual([]);
  });

  it("flags offline Ollama", () => {
    const issues = modelIssues(health({ ollama: "offline" }));
    expect(issues.join(" ")).toMatch(/offline/);
  });

  it("names the missing models", () => {
    const issues = modelIssues(health({ missing_models: ["llama3.2:3b", "nomic-embed-text"] }));
    expect(issues.join(" ")).toMatch(/llama3\.2:3b/);
    expect(issues.join(" ")).toMatch(/nomic-embed-text/);
  });

  it("flags a non-working answer model", () => {
    const issues = modelIssues(health({ answer_ready: false }));
    expect(issues.join(" ")).toMatch(/answer model/);
  });

  it("falls back to answer_key when answer_ready is absent (old backend)", () => {
    const h = health({ answer_key: "missing" });
    delete (h.components as Partial<Health["components"]>).answer_ready;
    delete (h.components as Partial<Health["components"]>).missing_models;
    expect(modelIssues(h).join(" ")).toMatch(/answer model/);
  });
});
