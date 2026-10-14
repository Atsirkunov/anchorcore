import { describe, expect, it } from "vitest";
import { basename, docTypeLabel, kindLabel, needsLook, shortRef, statusLabel } from "./entityLabels";

describe("entityLabels (R31.2 open-set taxonomy)", () => {
  it("labels the current kinds in plain language", () => {
    expect(kindLabel("decision")).toBe("Decision");
    expect(kindLabel("action")).toBe("To-do");
    expect(kindLabel("note")).toBe("Note");
    // legacy kind passes through, not dropped
    expect(kindLabel("document")).toBe("Document");
  });

  it("passes unknown kinds through instead of blanking them", () => {
    expect(kindLabel("risk")).toBe("Risk");
    expect(kindLabel("follow_up")).toBe("Follow up");
  });

  it("labels the R33.6 buckets", () => {
    expect(docTypeLabel("record")).toBe("Record");
    expect(docTypeLabel("reference")).toBe("Reference");
    expect(docTypeLabel("code")).toBe("Code");
  });

  it("renders legacy 8-way doc_types until re-sync", () => {
    expect(docTypeLabel("documentation")).toBe("Documentation");
    expect(docTypeLabel("decision_log")).toBe("Decision log");
    expect(docTypeLabel("runbook")).toBe("Runbook");
  });

  it("returns null when the backend sent no doc_type", () => {
    expect(docTypeLabel(undefined)).toBeNull();
    expect(docTypeLabel(null)).toBeNull();
    expect(docTypeLabel("")).toBeNull();
  });

  it("states trust consequences instead of claiming a human check", () => {
    expect(statusLabel("unverified")).toBe("New");
    expect(statusLabel("verified")).toBe("In answers");
    expect(statusLabel("disputed")).toBe("Disputed");
    expect(statusLabel("stale")).toBe("Stale");
  });

  it("queues never-seen and contested rows, nothing else", () => {
    expect(needsLook("unverified")).toBe(true);
    expect(needsLook("disputed")).toBe(true);
    expect(needsLook("verified")).toBe(false);
    expect(needsLook("stale")).toBe(false);
    expect(needsLook("anything-else")).toBe(false);
  });

  it("keeps relative refs verbatim, collapses absolute machine paths", () => {
    expect(shortRef("meetings/m1-sprint12-retro.md")).toBe("meetings/m1-sprint12-retro.md");
    expect(shortRef("/Users/ao/corpus/sample/meetings/m1.md")).toBe("meetings/m1.md");
    expect(shortRef("C:\\Users\\ao\\corpus\\m1.md")).toBe("corpus/m1.md");
    expect(basename("meetings/m1-sprint12-retro.md")).toBe("m1-sprint12-retro.md");
  });
});
