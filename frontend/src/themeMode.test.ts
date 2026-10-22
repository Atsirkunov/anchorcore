import { describe, expect, it } from "vitest";
import { parseMode, resolveTheme } from "./themeMode";

describe("theme mode", () => {
  it("treats anything unknown as auto", () => {
    expect(parseMode(null)).toBe("auto");
    expect(parseMode("sepia")).toBe("auto");
    expect(parseMode("dark")).toBe("dark");
  });

  it("auto follows the OS, explicit modes win", () => {
    expect(resolveTheme("auto", true)).toBe("dark");
    expect(resolveTheme("auto", false)).toBe("light");
    expect(resolveTheme("light", true)).toBe("light");
    expect(resolveTheme("dark", false)).toBe("dark");
  });
});
