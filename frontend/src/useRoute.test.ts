import { describe, expect, it } from "vitest";
import { parseRoute, routeHash } from "./useRoute";

describe("parseRoute (Wave 1)", () => {
  it("reads the tab from the hash", () => {
    expect(parseRoute("#/sources")).toBe("sources");
    expect(parseRoute("#/system")).toBe("system");
    expect(parseRoute("#/settings/answer-model")).toBe("settings");
    expect(parseRoute("#/Records?x=1")).toBe("records");
  });

  it("falls back to Ask for empty or unknown hashes", () => {
    expect(parseRoute("")).toBe("ask");
    expect(parseRoute("#")).toBe("ask");
    expect(parseRoute("#/nope")).toBe("ask");
    expect(parseRoute("#cite-3")).toBe("ask");
  });

  it("round-trips through routeHash", () => {
    expect(parseRoute(routeHash("records"))).toBe("records");
  });
});
