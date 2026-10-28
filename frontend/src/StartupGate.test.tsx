import { act, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { StartupGate } from "./StartupGate";
import type { StartupStatus } from "./startupText";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const STARTING: StartupStatus = {
  status: "starting",
  phase: "migrations",
  step: 2,
  of: 4,
  detail: "Migration 29: rewriting the search index",
  started_at: "2026-10-28T09:00:00Z",
  upgrading: true,
  version: "1.6.3",
  progress: null,
};

const FAILED: StartupStatus = {
  ...STARTING,
  status: "failed",
  phase: "failed",
  error: "migration 29 failed: disk I/O error",
  data_state: "Migration 29 was rolled back as one transaction, so it changed nothing: your library is at schema version 28.",
  log_file: "~/.anchorcore/anchorcore.log",
  support: "AnchorCore 1.6.3 on macos (aarch64): database upgrade failed at step 2 of 4",
};

let root: Root | null = null;
let container: HTMLDivElement | null = null;

async function render(node: ReactNode) {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  await act(async () => {
    root!.render(node);
  });
  // first probe: a 0 ms timer + the mocked fetch
  await act(async () => {
    await new Promise((r) => setTimeout(r, 5));
  });
  return container;
}

function respond(status: number, body: unknown) {
  return Promise.resolve(new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } }));
}

afterEach(async () => {
  vi.restoreAllMocks();
  vi.useRealTimers();
  if (root) {
    await act(async () => {
      root!.unmount();
    });
    root = null;
  }
  container?.remove();
  container = null;
});

describe("StartupGate (R41.7)", () => {
  it("shows the upgrade screen with the step, then loads the app when /health turns 200", async () => {
    const fetchMock = vi
      .spyOn(globalThis, "fetch")
      .mockImplementationOnce(() => respond(503, { ...STARTING, progress: { done: 3, total: 10, unit: "items" } }))
      .mockImplementation(() => respond(200, { status: "ok" }));
    const host = await render(
      <StartupGate pollMs={10}>
        <p>the app</p>
      </StartupGate>,
    );
    expect(host.textContent).toContain("Upgrading your library");
    expect(host.textContent).toContain("Step 2 of 4: Migration 29: rewriting the search index");
    expect(host.textContent).toContain("3 of 10 items");
    expect(host.textContent).toContain("Don't close AnchorCore");
    expect(host.querySelector('[role="progressbar"]')?.getAttribute("aria-valuenow")).toBe("30");
    expect(host.textContent).not.toContain("the app");
    await act(async () => {
      await new Promise((r) => setTimeout(r, 30));
    });
    expect(host.textContent).toContain("the app");
    expect(fetchMock).toHaveBeenCalledWith("/health", expect.anything());
  });

  it("shows the error, the data state, the log path and the support text on failure — and stops polling", async () => {
    const fetchMock = vi.spyOn(globalThis, "fetch").mockImplementation(() => respond(503, FAILED));
    const host = await render(
      <StartupGate pollMs={10}>
        <p>the app</p>
      </StartupGate>,
    );
    expect(host.querySelector('[role="alert"]')?.textContent).toContain("disk I/O error");
    expect(host.textContent).toContain("rolled back as one transaction");
    expect(host.textContent).toContain("~/.anchorcore/anchorcore.log");
    expect(host.textContent).toContain("send this to support");
    expect(host.textContent).toContain("AnchorCore 1.6.3 on macos");
    await act(async () => {
      await new Promise((r) => setTimeout(r, 30));
    });
    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(host.textContent).not.toContain("the app");
  });

  it("renders the app straight away on a normal start", async () => {
    vi.spyOn(globalThis, "fetch").mockImplementation(() => respond(200, { status: "ok" }));
    const host = await render(
      <StartupGate>
        <p>the app</p>
      </StartupGate>,
    );
    expect(host.textContent).toContain("the app");
  });
});
