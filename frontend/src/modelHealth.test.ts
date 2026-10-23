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

  it("flags offline Ollama with local answers down", () => {
    const issues = modelIssues(health({ ollama: "offline", answer_ready: false }));
    expect(issues.join(" ")).toMatch(/local answers stay basic/);
    expect(issues.join(" ")).toMatch(/keyword-only/);
  });

  it("narrows to search when answers come from an agent or cloud model", () => {
    const issues = modelIssues(health({ ollama: "offline" }));
    expect(issues.join(" ")).toMatch(/answers are unaffected/);
    expect(issues.join(" ")).toMatch(/keyword-only/);
    expect(issues.join(" ")).not.toMatch(/stay basic/);
  });

  it("stays silent when cloud answers and cloud embeddings cover Ollama", () => {
    const issues = modelIssues(health({ ollama: "offline", embedder: { provider: "cloud" } }));
    expect(issues).toEqual([]);
  });

  it("narrows to answers when only search is covered by cloud embeddings", () => {
    const issues = modelIssues(
      health({ ollama: "offline", answer_ready: false, embedder: { provider: "cloud" } }),
    );
    expect(issues.join(" ")).toMatch(/local answers stay basic/);
    expect(issues.join(" ")).not.toMatch(/keyword-only/);
  });

  it("names the missing models", () => {
    const issues = modelIssues(health({ missing_models: ["llama3.2:3b", "nomic-embed-text"] }));
    expect(issues.join(" ")).toMatch(/llama3\.2:3b/);
    expect(issues.join(" ")).toMatch(/nomic-embed-text/);
  });

  it("asks for search quality, not answers, when an answer model is ready", () => {
    const issues = modelIssues(health({ missing_models: ["nomic-embed-text"] }));
    expect(issues.join(" ")).toMatch(/search quality/);
    expect(issues.join(" ")).not.toMatch(/full answers/);
  });

  it("asks for full answers when no answer model is ready", () => {
    const issues = modelIssues(health({ missing_models: ["nomic-embed-text"], answer_ready: false }));
    expect(issues.join(" ")).toMatch(/full answers/);
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
