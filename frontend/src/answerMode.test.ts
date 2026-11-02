import { describe, expect, it } from "vitest";
import { isAgentCompose, isContextOnly, isEvidenceView, modeBannerNotes } from "./answerMode";

describe("modeBannerNotes (B58)", () => {
  it("is silent for composed hybrid answers", () => {
    expect(modeBannerNotes("composed", "hybrid")).toEqual([]);
  });

  it("leaves context-only answers to the evidence view", () => {
    expect(modeBannerNotes("context_only", "hybrid")).toEqual([]);
  });

  it("flags keyword-only retrieval", () => {
    const notes = modeBannerNotes("composed", "keyword_only");
    expect(notes).toHaveLength(1);
    expect(notes[0]).toContain("keywords only");
  });

  it("keeps the keyword-only note on context-only answers", () => {
    expect(modeBannerNotes("context_only", "keyword_only")).toHaveLength(1);
  });

  it("stays silent on unknown or missing flags (older backend)", () => {
    expect(modeBannerNotes(undefined, undefined)).toEqual([]);
    expect(modeBannerNotes("none", "hybrid")).toEqual([]);
    expect(modeBannerNotes("whatever", "whatever")).toEqual([]);
  });
});

describe("isAgentCompose (R45.2)", () => {
  it("is true only for agent_compose", () => {
    expect(isAgentCompose("agent_compose")).toBe(true);
    expect(isAgentCompose("composed")).toBe(false);
    expect(isAgentCompose("context_only")).toBe(false);
    expect(isAgentCompose(undefined)).toBe(false);
  });

  it("agent_compose raises no degraded banner", () => {
    expect(modeBannerNotes("agent_compose", "hybrid")).toEqual([]);
  });
});

describe("isEvidenceView (Wave 1)", () => {
  it("covers agent mode and context-only local answers", () => {
    expect(isEvidenceView("agent_compose")).toBe(true);
    expect(isEvidenceView("context_only")).toBe(true);
    expect(isContextOnly("context_only")).toBe(true);
  });

  it("renders composed, none and unknown modes as answers", () => {
    expect(isEvidenceView("composed")).toBe(false);
    expect(isEvidenceView("none")).toBe(false);
    expect(isEvidenceView(undefined)).toBe(false);
  });
});
