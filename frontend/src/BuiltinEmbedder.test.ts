import { describe, expect, it } from "vitest";
import { downloadPct, embedderLine } from "./builtinEmbedderText";
import type { EmbedModelStatus } from "./types";

const base: EmbedModelStatus = {
  model: "nomic-embed-text-v1.5",
  revision: "e9b6763023c676ca8431644204f50c2b100d9aab",
  setting: "auto",
  active: "ollama",
  ready: false,
  dir: "/data/models/nomic-embed-text-v1.5",
  download_bytes: 547_652_102,
  backend: null,
  progress: { phase: "idle", done_bytes: 0, total_bytes: 0, file: null, error: null },
};

describe("BuiltinEmbedder helpers", () => {
  it("computes download percentage, full once converting", () => {
    expect(downloadPct({ ...base.progress, phase: "downloading", done_bytes: 50, total_bytes: 200 })).toBe(25);
    expect(downloadPct({ ...base.progress, phase: "downloading", total_bytes: 0 })).toBe(0);
    expect(downloadPct({ ...base.progress, phase: "converting" })).toBe(100);
  });

  it("describes where search embeddings come from", () => {
    expect(embedderLine(base)).toMatch(/come from Ollama/);
    expect(embedderLine({ ...base, ready: true, active: "builtin", backend: "metal" })).toMatch(/ready .*metal.*no Ollama/);
    expect(embedderLine({ ...base, setting: "builtin" })).toMatch(/keyword-only/);
    expect(embedderLine({ ...base, ready: true, setting: "ollama" })).toMatch(/Settings say so/);
    expect(embedderLine({ ...base, ready: true })).toMatch(/Ollama is serving/);
  });
});
