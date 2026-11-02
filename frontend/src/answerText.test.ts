import { describe, expect, it } from "vitest";
import { excerpt, parseAnswer, parseInline, relativeTo, splitSourceRef, stripMatchMarks } from "./answerText";

describe("stripMatchMarks", () => {
  it("drops mark lines and inline marks", () => {
    expect(stripMatchMarks("a\n[match 7]\nOwner: Sarah.\n[/match 7]\nb")).toBe("a\nOwner: Sarah.\nb");
    expect(stripMatchMarks("as stated in [S1] [match 7].")).toBe("as stated in [S1].");
  });
});

describe("parseInline", () => {
  it("splits bold and citation markers", () => {
    expect(parseInline("See **flag** in [S2].")).toEqual([
      { t: "text", v: "See " },
      { t: "bold", v: "flag" },
      { t: "text", v: " in " },
      { t: "cite", n: 2 },
      { t: "text", v: "." },
    ]);
  });
});

describe("parseAnswer", () => {
  it("builds paragraphs and numbered lists", () => {
    const blocks = parseAnswer("Owned by Sarah [S1].\n\nSteps:\n1. **Flag** back [S2]\n2. Monitor");
    expect(blocks.map((b) => b.t)).toEqual(["p", "p", "ol"]);
    const list = blocks[2];
    expect(list.t === "ol" && list.items.length).toBe(2);
  });

  it("renders headings as bold run-ins, never #", () => {
    const blocks = parseAnswer("## Decision\nUse ledger v2 [S1].\n\n# Steps");
    expect(blocks).toEqual([
      { t: "p", parts: [{ t: "bold", v: "Decision" }, { t: "text", v: " " }, { t: "text", v: "Use ledger v2 " }, { t: "cite", n: 1 }, { t: "text", v: "." }] },
      { t: "p", parts: [{ t: "bold", v: "Steps" }] },
    ]);
  });

  it("keeps one ordered list across blank lines and honours its start", () => {
    const blocks = parseAnswer("1. Flag\n\n1. Drain\n\n1. Monitor");
    expect(blocks).toHaveLength(1);
    const list = blocks[0];
    expect(list.t === "ol" && list.items.length).toBe(3);
    expect(list.t === "ol" && list.start).toBe(1);
    const later = parseAnswer("3. Third\n4. Fourth")[0];
    expect(later.t === "ol" && later.start).toBe(3);
  });

  it("re-flows hard-wrapped paragraphs and list items", () => {
    const blocks = parseAnswer("The webhook\ndelivery retries.\n\n- first item that\nwraps here\n- second");
    expect(blocks[0]).toEqual({ t: "p", parts: [{ t: "text", v: "The webhook delivery retries." }] });
    const list = blocks[1];
    expect(list.t === "ul" && list.items.length).toBe(2);
    expect(JSON.stringify(list)).toContain("wraps here");
  });

  it("ends a list at a paragraph after a blank line", () => {
    expect(parseAnswer("- a\n\nAfter.").map((b) => b.t)).toEqual(["ul", "p"]);
  });

  it("never shows match marks", () => {
    const text = JSON.stringify(parseAnswer("Owned by Sarah, as stated in [S1] [match 7]."));
    expect(text).not.toContain("match");
  });
});

describe("excerpt", () => {
  it("strips marks and markdown, collapses whitespace, caps length", () => {
    expect(excerpt("[match 7]\n# Decision: X\n\n**Owner:** Sarah.\n[/match 7]")).toBe("Decision: X Owner: Sarah.");
    expect(excerpt("w ".repeat(400), 20).endsWith("…")).toBe(true);
  });
});

describe("source locations", () => {
  it("splits the section suffix", () => {
    expect(splitSourceRef("/x/sample/specs/r.md §2")).toEqual({ location: "/x/sample/specs/r.md", section: "2" });
    expect(splitSourceRef("/x/a.md")).toEqual({ location: "/x/a.md", section: null });
  });

  it("is relative to the longest matching folder root", () => {
    expect(relativeTo("/x/sample/specs/r.md", ["/x", "/x/sample/"])).toBe("specs/r.md");
    expect(relativeTo("C:\\data\\notes\\a.md", ["C:\\data"])).toBe("notes\\a.md");
    expect(relativeTo("jira:ABC-1", ["/x"])).toBe("jira:ABC-1");
  });
});
