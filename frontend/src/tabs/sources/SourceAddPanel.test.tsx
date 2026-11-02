import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { Source } from "../../types";
import { SourceAddPanel } from "./SourceAddPanel";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let root: Root | null = null;
let container: HTMLDivElement | null = null;

const SOURCE: Source = {
  id: 7,
  connector: "folder",
  name: "Handbook",
  enabled: true,
  label: "internal",
  last_synced_at: null,
  last_error: null,
  error_count: 0,
  created_at: "2026-11-02T00:00:00+00:00",
};

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } });
}

function renderPanel(onClose: () => void, onAdded: (e: string | null) => void) {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  return act(async () => {
    root!.render(
      <QueryClientProvider client={new QueryClient()}>
        <SourceAddPanel onClose={onClose} onAdded={onAdded} />
      </QueryClientProvider>,
    );
  });
}

function setInput(el: HTMLInputElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, "value")!.set!;
  setter.call(el, value);
  el.dispatchEvent(new Event("input", { bubbles: true }));
}

function button(label: string): HTMLButtonElement {
  const el = [...container!.querySelectorAll("button")].find((b) => b.textContent === label);
  if (!el) throw new Error(`button "${label}" not found`);
  return el as HTMLButtonElement;
}

afterEach(async () => {
  vi.restoreAllMocks();
  if (root) {
    await act(async () => {
      root!.unmount();
    });
    root = null;
  }
  container?.remove();
  container = null;
});

describe("SourceAddPanel (UX proposal §Library)", () => {
  it("labels every field and defaults the visibility to Internal", async () => {
    await renderPanel(() => {}, () => {});
    const text = container!.textContent ?? "";
    for (const label of ["Type", "Folder", "Name", "Who may see it", "Internal", "Public", "Sensitive"]) {
      expect(text).toContain(label);
    }
    const checked = container!.querySelector<HTMLInputElement>('input[name="source-label"]:checked');
    expect(checked?.value).toBe("internal");
  });

  it("creates the source, then starts its first sync", async () => {
    const calls: { url: string; method: string; body: string }[] = [];
    vi.spyOn(globalThis, "fetch").mockImplementation(async (url, init) => {
      const u = String(url);
      if (u === "/csrf") return json({ csrf_token: "t" });
      const method = (init?.method ?? "GET").toUpperCase();
      calls.push({ url: u, method, body: String(init?.body ?? "") });
      if (u === "/sources" && method === "POST") return json(SOURCE);
      if (u === "/sources/7/sync" && method === "POST") return json({ id: 3, status: "pending" });
      return json({}, 404);
    });
    const onAdded = vi.fn();
    await renderPanel(() => {}, onAdded);
    await act(async () => {
      setInput(container!.querySelector<HTMLInputElement>('input[aria-label="Folder path"]')!, "/Users/me/Documents/Handbook");
      setInput(container!.querySelector<HTMLInputElement>("#add-source-name")!, "Handbook");
    });
    await act(async () => {
      button("Add and sync").click();
    });
    for (let i = 0; i < 50 && onAdded.mock.calls.length === 0; i++) {
      await act(async () => {
        await new Promise((r) => setTimeout(r, 10));
      });
    }
    expect(onAdded).toHaveBeenCalledWith(null);
    expect(calls.map((c) => `${c.method} ${c.url}`)).toEqual(["POST /sources", "POST /sources/7/sync"]);
    expect(JSON.parse(calls[0].body)).toMatchObject({ connector: "folder", name: "Handbook", label: "internal", config: { path: "/Users/me/Documents/Handbook" } });
  });

  it("closes instead of stranding the panel when the sync start fails", async () => {
    vi.spyOn(globalThis, "fetch").mockImplementation(async (url) => {
      const u = String(url);
      if (u === "/csrf") return json({ csrf_token: "t" });
      if (u === "/sources") return json(SOURCE);
      if (u === "/sources/7/sync") return json({ detail: "sync already active for source 7 (job 9)" }, 409);
      return json({}, 404);
    });
    const onAdded = vi.fn();
    await renderPanel(() => {}, onAdded);
    await act(async () => {
      button("Add and sync").click();
    });
    for (let i = 0; i < 50 && onAdded.mock.calls.length === 0; i++) {
      await act(async () => {
        await new Promise((r) => setTimeout(r, 10));
      });
    }
    expect(onAdded).toHaveBeenCalledWith("sync already active for source 7 (job 9)");
  });

  it("closes on ×", async () => {
    const onClose = vi.fn();
    await renderPanel(onClose, () => {});
    await act(async () => {
      button("×").click();
    });
    for (let i = 0; i < 50 && onClose.mock.calls.length === 0; i++) {
      await act(async () => {
        await new Promise((r) => setTimeout(r, 10));
      });
    }
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("closes on Escape", async () => {
    const onClose = vi.fn();
    await renderPanel(onClose, () => {});
    await act(async () => {
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    });
    for (let i = 0; i < 50 && onClose.mock.calls.length === 0; i++) {
      await act(async () => {
        await new Promise((r) => setTimeout(r, 10));
      });
    }
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});
