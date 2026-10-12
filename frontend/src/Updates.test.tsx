import { act, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { api } from "./api";
import { UpdateBanner, UpdateCard } from "./Updates";
import type { UpdateStatus } from "./types";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const IDLE: UpdateStatus = {
  current: "1.0.21",
  latest: null,
  update_available: false,
  release_url: "https://github.com/Atsirkunov/anchorcore/releases/latest",
  published_at: null,
  asset_url: null,
  checked_at: null,
  check_enabled: false,
  error: null,
};

const AVAILABLE: UpdateStatus = {
  ...IDLE,
  latest: "1.0.22",
  update_available: true,
  published_at: "2026-10-12T07:00:00Z",
  asset_url: "https://github.com/Atsirkunov/anchorcore/releases/latest/download/AnchorCore-windows.zip",
  checked_at: "2026-10-12T09:31:00Z",
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
  await act(async () => {});
  return container;
}

afterEach(async () => {
  vi.restoreAllMocks();
  localStorage.clear();
  if (root) {
    await act(async () => {
      root!.unmount();
    });
    root = null;
  }
  container?.remove();
  container = null;
});

function buttons(host: HTMLElement, label: string): HTMLButtonElement[] {
  return [...host.querySelectorAll("button")].filter((b) => b.textContent?.includes(label)) as HTMLButtonElement[];
}

describe("Updates card", () => {
  it("loads the cached status, then only the button performs the check", async () => {
    const statusSpy = vi.spyOn(api, "updateStatus").mockResolvedValue(IDLE);
    const host = await render(<UpdateCard />);
    // loading the card is a cache read (no `refresh`) — the trust contract
    expect(statusSpy).toHaveBeenCalledWith();
    expect(host.textContent).toContain("current 1.0.21");
    expect(host.textContent).toContain("Startup checks: off");

    statusSpy.mockResolvedValue(AVAILABLE);
    await act(async () => {
      buttons(host, "Check for updates")[0].click();
    });
    await act(async () => {});
    expect(statusSpy).toHaveBeenLastCalledWith(true);
    expect(host.textContent).toContain("AnchorCore 1.0.22 is available");
    const download = [...host.querySelectorAll("a")].find((a) => a.textContent?.includes("Download"));
    expect(download?.getAttribute("href")).toContain("AnchorCore-windows.zip");
  });

  it("shows a failed check without breaking the card", async () => {
    vi.spyOn(api, "updateStatus").mockResolvedValue({
      ...IDLE,
      checked_at: "2026-10-12T09:31:00Z",
      error: "couldn't reach github.com (offline?)",
    });
    const host = await render(<UpdateCard />);
    expect(host.textContent).toContain("couldn't reach github.com");
  });
});

describe("Updates banner", () => {
  it("stays hidden when nothing newer is known", async () => {
    vi.spyOn(api, "updateStatus").mockResolvedValue(IDLE);
    const host = await render(<UpdateBanner onOpen={() => {}} />);
    expect(host.textContent).toBe("");
  });

  it("shows an already-completed check and dismisses per version", async () => {
    vi.spyOn(api, "updateStatus").mockResolvedValue(AVAILABLE);
    const onOpen = vi.fn();
    const host = await render(<UpdateBanner onOpen={onOpen} />);
    expect(host.textContent).toContain("AnchorCore 1.0.22 is available");
    await act(async () => {
      buttons(host, "View")[0].click();
    });
    expect(onOpen).toHaveBeenCalled();
    await act(async () => {
      buttons(host, "Dismiss")[0].click();
    });
    await act(async () => {});
    expect(host.textContent).toBe("");
    expect(localStorage.getItem("update-dismissed")).toBe("1.0.22");
  });
});
