import { afterEach, describe, expect, it, vi } from "vitest";
import { RequestAbortedError } from "./api";

describe("api request plumbing", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("throws RequestAbortedError when the caller aborts", async () => {
    const controller = new AbortController();
    const { api } = await import("./api");
    // fetch that never resolves unless aborted
    vi.stubGlobal(
      "fetch",
      vi.fn((_url: string, init?: RequestInit) => {
        return new Promise((_resolve, reject) => {
          init?.signal?.addEventListener("abort", () => reject(new DOMException("Aborted", "AbortError")));
        });
      }),
    );
    const promise = api.ask("question", [], undefined, controller.signal);
    controller.abort();
    await expect(promise).rejects.toBeInstanceOf(RequestAbortedError);
  });

  it("parses FastAPI detail strings from error responses", async () => {
    const { api } = await import("./api");
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue({
        ok: false,
        status: 422,
        text: async () => JSON.stringify({ detail: "project name is required" }),
      }),
    );
    await expect(api.createProject({ name: " " })).rejects.toThrow("project name is required");
  });

  it("surfaces non-JSON error bodies", async () => {
    const { api } = await import("./api");
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue({
        ok: false,
        status: 500,
        text: async () => "internal server error text",
      }),
    );
    await expect(api.health()).rejects.toThrow(/internal server error text/);
  });
});
