import { describe, expect, it } from "vitest";
import { PLACEHOLDER, stripPlaceholders } from "./placeholders";

describe("stripPlaceholders", () => {
  it("removes the ***set*** masked placeholder so it never overwrites a stored secret", () => {
    const payload = {
      answer_api_key: "***set***",
      classifier_api_key: "",
      embed_api_key: "***set***",
      answer_model: "gpt-4o-mini",
    };
    const cleaned = stripPlaceholders(payload);
    expect(cleaned.answer_api_key).toBeUndefined();
    expect(cleaned.embed_api_key).toBeUndefined();
    expect(cleaned.answer_model).toBe("gpt-4o-mini");
  });

  it("keeps empty strings (an explicit empty value clears the secret, not the placeholder)", () => {
    const cleaned = stripPlaceholders({ token: "", api_key: "***set***" });
    expect(cleaned.token).toBe("");
    expect(cleaned.api_key).toBeUndefined();
  });

  it("leaves non-placeholder values untouched", () => {
    const cleaned = stripPlaceholders({ path: "/tmp/x", jira: { token: "real" } });
    expect(cleaned).toEqual({ path: "/tmp/x", jira: { token: "real" } });
  });

  it("exports the exact placeholder constant shared with the UI", () => {
    expect(PLACEHOLDER).toBe("***set***");
  });
});
