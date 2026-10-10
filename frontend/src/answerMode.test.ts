import { describe, expect, it } from "vitest";
import { modeBannerNotes } from "./answerMode";

describe("modeBannerNotes (B58)", () => {
  it("is silent for composed hybrid answers", () => {
    expect(modeBannerNotes("composed", "hybrid")).toEqual([]);
  });

  it("flags context-only answers", () => {
    const notes = modeBannerNotes("context_only", "hybrid");
    expect(notes).toHaveLength(1);
    expect(notes[0]).toContain("no answer model ran");
  });

  it("flags keyword-only retrieval", () => {
    const notes = modeBannerNotes("composed", "keyword_only");
    expect(notes).toHaveLength(1);
    expect(notes[0]).toContain("keywords only");
  });

  it("stacks both notes when fully degraded", () => {
    expect(modeBannerNotes("context_only", "keyword_only")).toHaveLength(2);
  });

  it("stays silent on unknown or missing flags (older backend)", () => {
    expect(modeBannerNotes(undefined, undefined)).toEqual([]);
    expect(modeBannerNotes("none", "hybrid")).toEqual([]);
    expect(modeBannerNotes("whatever", "whatever")).toEqual([]);
  });
});
