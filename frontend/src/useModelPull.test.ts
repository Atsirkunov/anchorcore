import { describe, expect, it } from "vitest";
import { summarizePulls } from "./useModelPull";
import type { PullProgress } from "./types";

function row(model: string, over: Partial<PullProgress> = {}): PullProgress {
  return { model, status: "pulling", completed: 0, total: 0, done: false, error: null, ...over };
}

describe("summarizePulls", () => {
  it("ignores rows for models nobody asked about", () => {
    const s = summarizePulls([row("other:1", { done: true })], ["llama3.2:3b"]);
    expect(s).toEqual({ total: 1, done: 0, failed: [], pct: 0, busy: true });
  });

  it("computes byte-weighted percent across models", () => {
    const s = summarizePulls(
      [row("a", { completed: 50, total: 100 }), row("b", { completed: 25, total: 100 })],
      ["a", "b"],
    );
    expect(s.pct).toBe(38); // 75/200 rounded
    expect(s.busy).toBe(true);
  });

  it("reports done and failed models separately", () => {
    const s = summarizePulls(
      [row("a", { done: true, status: "success" }), row("b", { done: true, error: "no such model" })],
      ["a", "b"],
    );
    expect(s.done).toBe(1);
    expect(s.failed).toEqual(["b"]);
    expect(s.busy).toBe(false);
  });

  it("stays at 0% with no byte totals yet", () => {
    const s = summarizePulls([row("a")], ["a"]);
    expect(s.pct).toBe(0);
    expect(s.busy).toBe(true);
  });
});
