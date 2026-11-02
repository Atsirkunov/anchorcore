import { afterEach, describe, expect, it, vi } from "vitest";
import { RequestAbortedError, RequestTimeoutError } from "./api";

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

  it("identifies the app with the UI client header (R35.5 gate exemption)", async () => {
    const { api } = await import("./api");
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({ status: "ok" }),
      blob: async () => new Blob(["x"]),
    });
    vi.stubGlobal("fetch", fetchMock);
    await api.health();
    expect((fetchMock.mock.calls[0][1]?.headers as Record<string, string>)["X-Anchor-Client"]).toBe("ui");
    await api.assetBlob("ab12");
    expect(fetchMock.mock.calls[1][0]).toBe("/media/ab12");
    expect((fetchMock.mock.calls[1][1]?.headers as Record<string, string>)["X-Anchor-Client"]).toBe("ui");
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
  // R48.7 ----------------------------------------------------------------
  const neverResolves = (_url: string, init?: RequestInit) =>
    new Promise<Response>((_resolve, reject) => {
      init?.signal?.addEventListener("abort", () => reject(new DOMException("Aborted", "AbortError")), { once: true });
    });

  it("reports a timeout as a timeout, not a cancel", async () => {
    vi.useFakeTimers();
    try {
      const { api } = await import("./api");
      vi.stubGlobal("fetch", vi.fn(neverResolves));
      const promise = api.health();
      const settled = expect(promise).rejects.toBeInstanceOf(RequestTimeoutError);
      await vi.advanceTimersByTimeAsync(30_000);
      await settled;
    } finally {
      vi.useRealTimers();
    }
  });

  it("gives ask longer than the default timeout", async () => {
    vi.useFakeTimers();
    try {
      const { api, ASK_TIMEOUT_MS } = await import("./api");
      vi.stubGlobal(
        "fetch",
        vi.fn((url: string, init?: RequestInit) =>
          url === "/csrf" ? Promise.resolve({ ok: true, json: async () => ({ csrf_token: "t" }) } as Response) : neverResolves(url, init),
        ),
      );
      let state = "pending";
      const promise = api.ask("q").catch((e) => {
        state = e instanceof RequestTimeoutError ? "timeout" : "other";
      });
      await vi.advanceTimersByTimeAsync(60_000);
      expect(state).toBe("pending");
      await vi.advanceTimersByTimeAsync(ASK_TIMEOUT_MS);
      await promise;
      expect(state).toBe("timeout");
    } finally {
      vi.useRealTimers();
    }
  });

  const refusal = (status: number, detail: string) => {
    const res = { ok: false, status, text: async () => JSON.stringify({ detail }) } as Response;
    return { ...res, clone: () => res } as Response;
  };

  it("refreshes a stale CSRF token once and retries the write", async () => {
    const { api } = await import("./api");
    let token = "old";
    const fetchMock = vi.fn(async (url: string, init?: RequestInit) => {
      if (url === "/csrf") return { ok: true, json: async () => ({ csrf_token: token }) } as Response;
      const sent = (init?.headers as Record<string, string>)["X-CSRF-Token"];
      if (sent !== "new") return refusal(403, "Forbidden: missing or invalid CSRF token (X-CSRF-Token)");
      return { ok: true, json: async () => ({ id: 1 }) } as Response;
    });
    vi.stubGlobal("fetch", fetchMock);
    await api.createProject({ name: "warm-up" }).catch(() => undefined); // caches "old"
    token = "new"; // the server restarted
    await expect(api.createProject({ name: "p" })).resolves.toEqual({ id: 1 });
    const writes = fetchMock.mock.calls.filter(([url]) => url === "/projects");
    expect(writes.map(([, init]) => (init?.headers as Record<string, string>)["X-CSRF-Token"]).slice(-2)).toEqual(["old", "new"]);
  });

  it("does not retry a 403 that is not about CSRF", async () => {
    const { api } = await import("./api");
    const fetchMock = vi.fn(async (url: string) =>
      url === "/csrf" ? ({ ok: true, json: async () => ({ csrf_token: "t" }) } as Response) : refusal(403, "gated_provider"),
    );
    vi.stubGlobal("fetch", fetchMock);
    await expect(api.createProject({ name: "p" })).rejects.toThrow("gated_provider");
    expect(fetchMock.mock.calls.filter(([url]) => url === "/projects")).toHaveLength(1);
  });
});
