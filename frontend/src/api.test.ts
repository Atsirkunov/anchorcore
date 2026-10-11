import { afterEach, describe, expect, it, vi } from "vitest";
import { RequestAbortedError } from "./api";

describe("api request plumbing", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("throws RequestAbortedError when the caller aborts", async () => {
    const controller = new AbortController();
    const { api } = await import("./api");
    // fetch that never resolves unless aborted; handle CSRF fetch separately
    vi.stubGlobal(
      "fetch",
      vi.fn((url: string, init?: RequestInit) => {
        if (typeof url === "string" && url.includes("/csrf")) {
          return Promise.resolve({ ok: true, json: async () => ({ csrf_token: "test" }) } as Response);
        }
        return new Promise((_resolve, reject) => {
          if (init?.signal?.aborted) {
            reject(new DOMException("Aborted", "AbortError"));
            return;
          }
          init?.signal?.addEventListener("abort", () => reject(new DOMException("Aborted", "AbortError")), { once: true });
        });
      }),
    );
    const promise = api.ask("question", [], undefined, controller.signal);
    controller.abort();
    await expect(promise).rejects.toBeInstanceOf(RequestAbortedError);
  }, 10000);

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

  it("builds the folder-browse query string", async () => {
    const { api } = await import("./api");
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({ path: null, parent: null, home: null, dirs: [], truncated: false }),
    });
    vi.stubGlobal("fetch", fetchMock);
    await api.browseFolders("C:\\Users\\Aleksandr");
    expect(fetchMock.mock.calls[0][0]).toBe("/system/fs?path=C%3A%5CUsers%5CAleksandr");
    await api.browseFolders();
    expect(fetchMock.mock.calls[1][0]).toBe("/system/fs");
    await api.browseFolders("  ");
    expect(fetchMock.mock.calls[2][0]).toBe("/system/fs");
  });
});
