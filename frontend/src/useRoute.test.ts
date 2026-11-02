import { describe, expect, it } from "vitest";
import { parseRoute, routeHash, settingsSection } from "./useRoute";

describe("parseRoute (R49.1, R49.9)", () => {
  it("reads the tab and section from the hash", () => {
    expect(parseRoute("#/library")).toEqual({ tab: "library", sub: null });
    expect(parseRoute("#/library/12")).toEqual({ tab: "library", sub: "12" });
    expect(parseRoute("#/settings/answers")).toEqual({ tab: "settings", sub: "answers" });
    expect(parseRoute("#/Records?x=1")).toEqual({ tab: "records", sub: null });
    expect(parseRoute("#/agents")).toEqual({ tab: "agents", sub: null });
  });

  it("redirects the pre-R49.9 hashes", () => {
    expect(parseRoute("#/sources")).toEqual({ tab: "library", sub: null });
    expect(parseRoute("#/system")).toEqual({ tab: "settings", sub: "status" });
  });

  it("falls back to Ask for empty or unknown hashes", () => {
    expect(parseRoute("").tab).toBe("ask");
    expect(parseRoute("#").tab).toBe("ask");
    expect(parseRoute("#/nope").tab).toBe("ask");
    expect(parseRoute("#cite-3").tab).toBe("ask");
  });

  it("round-trips through routeHash", () => {
    expect(parseRoute(routeHash("settings", "backup"))).toEqual({ tab: "settings", sub: "backup" });
    expect(parseRoute(routeHash("library", 7))).toEqual({ tab: "library", sub: "7" });
    expect(routeHash("records")).toBe("#/records");
  });

  it("defaults unknown Settings sections to Answers", () => {
    expect(settingsSection("status")).toBe("status");
    expect(settingsSection(null)).toBe("answers");
    expect(settingsSection("nope")).toBe("answers");
  });
});
