import { describe, expect, it } from "vitest";
import type { AnswerModelOption } from "../types";
import { exceedsRam, initialPick, isMissing, ollamaAnswerBase, ramLabel, sameModel, stepLabel, stepsFor } from "./wizardText";

const G = 1024 * 1024 * 1024;
const opt = (model: string, min_ram_gb: number, recommended = false): AnswerModelOption => ({
  model,
  label: model,
  download_gb: 1,
  min_ram_gb,
  recommended,
});
const catalog = [opt("llama3.2:1b", 0), opt("llama3.2:3b", 8), opt("qwen2.5:7b", 16, true), opt("qwen2.5:14b", 32)];

describe("wizard steps", () => {
  it("local mode picks a model and offers an agent at the end", () => {
    expect(stepsFor("local")).toEqual(["setup", "answers", "source", "ask", "agent"]);
  });
  it("agent mode connects the agent instead of a model", () => {
    expect(stepsFor("agent")).toEqual(["setup", "agent", "source", "ask"]);
    expect(stepLabel("ask", "agent")).toBe("Try a search");
    expect(stepLabel("agent", "agent")).toBe("Connect your agent");
  });
});

describe("RAM-sized picker helpers", () => {
  it("labels RAM in whole GB", () => {
    expect(ramLabel(16 * G)).toBe("16 GB");
    expect(ramLabel(15.6 * G)).toBe("16 GB");
    expect(ramLabel(null)).toBeNull();
    expect(ramLabel(0)).toBeNull();
  });
  it("flags models above the machine's RAM, with firmware slack", () => {
    expect(exceedsRam(opt("qwen2.5:14b", 32), 16 * G)).toBe(true);
    expect(exceedsRam(opt("qwen2.5:7b", 16), 15.6 * G)).toBe(false);
    expect(exceedsRam(opt("qwen2.5:14b", 32), null)).toBe(false);
  });
  it("derives the answer URL from the Ollama base", () => {
    expect(ollamaAnswerBase("http://localhost:11434")).toBe("http://localhost:11434/v1");
    expect(ollamaAnswerBase("http://10.0.0.2:11434/")).toBe("http://10.0.0.2:11434/v1");
    expect(ollamaAnswerBase("http://x:1/v1")).toBe("http://x:1/v1");
    expect(ollamaAnswerBase("")).toBe("http://localhost:11434/v1");
  });
  it("matches model names like the server", () => {
    expect(sameModel("qwen2.5:7b", "Qwen2.5:7b")).toBe(true);
    expect(sameModel("qwen2.5:7b", "qwen2.5:14b")).toBe(false);
    expect(sameModel("nomic-embed-text", "nomic-embed-text:latest")).toBe(true);
    expect(sameModel("llama3.2", "llama3.2:3b")).toBe(true);
    expect(isMissing(["qwen2.5:14b"], "qwen2.5:7b")).toBe(false);
    expect(isMissing(["qwen2.5:14b"], "qwen2.5:14b")).toBe(true);
  });
  it("preselects the configured model, else the recommendation", () => {
    expect(initialPick(catalog, "llama3.2:3b", "qwen2.5:7b")).toBe("llama3.2:3b");
    expect(initialPick(catalog, "gpt-4o-mini", "qwen2.5:7b")).toBe("qwen2.5:7b");
    expect(initialPick(catalog, null, "qwen2.5:7b")).toBe("qwen2.5:7b");
  });
});
