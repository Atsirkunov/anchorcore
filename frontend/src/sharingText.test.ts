import { describe, expect, it } from "vitest";
import type { SensitiveSharing, SharingDestination } from "./types";
import { destinationStatus, sensitiveHint, sensitiveReach } from "./sharingText";

function dest(over: Partial<SharingDestination> & Pick<SharingDestination, "id">): SharingDestination {
  return { label: over.id, destination: "", local: false, env: false, switch: false, allowed: false, stale_for: null, ...over };
}

function state(over: Partial<SensitiveSharing> = {}): SensitiveSharing {
  return {
    embed: dest({ id: "embed", destination: "the built-in model on this computer", local: true, allowed: true }),
    answer: dest({ id: "answer", destination: "https://api.openai.com" }),
    agents: dest({ id: "agents", destination: "AI agents connected to this app (MCP / skill)" }),
    ...over,
  };
}

describe("R35.6 sensitive sharing text", () => {
  it("is local only by default — local providers never count as sharing", () => {
    expect(sensitiveReach(state())).toEqual([]);
    expect(sensitiveHint(state())).toContain("local models only");
    expect(sensitiveHint(undefined)).toContain("local models only");
  });

  it("names every allowed cloud host and agents", () => {
    const s = state({
      answer: dest({ id: "answer", destination: "https://api.openai.com", switch: true, allowed: true }),
      agents: dest({ id: "agents", destination: "x", switch: true, allowed: true }),
    });
    expect(sensitiveHint(s)).toBe("also sent to api.openai.com, AI agents (MCP / skill)");
  });

  it("explains env, stale and local states", () => {
    expect(destinationStatus(dest({ id: "answer", env: true, allowed: true }))).toContain("Set by environment");
    expect(destinationStatus(dest({ id: "answer", stale_for: "https://old.example" }))).toContain("old.example");
    expect(destinationStatus(dest({ id: "embed", local: true, allowed: true }))).toContain("no switch needed");
    expect(destinationStatus(dest({ id: "agents" }))).toContain("withheld");
  });
});
