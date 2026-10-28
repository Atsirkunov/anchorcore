import { describe, expect, it } from "vitest";
import { DEFAULT_SEARCH_HANDLE, handleError, initialHandle, installNote } from "./skillHandle";

describe("search skill handle (R47.4)", () => {
  it("accepts one folder name and rejects anything path-like", () => {
    for (const ok of ["ask-docs", "kb", "team-memory-2"]) expect(handleError(ok)).toBeNull();
    for (const bad of ["", "-x", "x-", "Ask", "a b", "a/b", "..", "a.b", "a".repeat(65)]) expect(handleError(bad)).not.toBeNull();
    expect(handleError("anchorcore-setup")).toContain("setup skill");
  });

  it("defaults to what that agent already has, else ask-docs", () => {
    expect(initialHandle(undefined)).toBe(DEFAULT_SEARCH_HANDLE);
    expect(initialHandle([])).toBe("ask-docs");
    expect(initialHandle(["kb"])).toBe("kb");
  });

  it("names the handle and any retired copy", () => {
    const note = installNote(
      { ok: true, path: "/h/.claude/skills/kb/SKILL.md", handle: "kb", retired: ["/h/.claude/skills/ask-docs/SKILL.md.anchorcore.bak"] },
      "",
      "Claude Code",
    );
    expect(note).toBe("Installed as /kb to /h/.claude/skills/kb/SKILL.md. Retired 1 older copy (kept as .anchorcore.bak). Restart Claude Code to pick it up.");
    expect(installNote({ ok: false, error: "'notes' already holds a different skill" }, "", "x")).toContain("different skill");
  });
});
