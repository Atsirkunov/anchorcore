import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { api } from "../../api";
import type { MediaList, MediaRow } from "../../types";
import { UnplacedImages } from "./UnplacedImages";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let root: Root | null = null;
let container: HTMLDivElement | null = null;

const row: MediaRow = {
  asset_id: "a1",
  item_id: 7,
  source_id: 1,
  source_name: "docs",
  origin: "file",
  src: "flow.png",
  path: "flow.png",
  mime: "image/png",
  w: 64,
  h: 48,
  size: 120,
  url: "/media/a1",
  alt: "",
  caption: "",
  context: "",
  attribution: "none",
  manual_caption: "",
  target: null,
  updated_at: "2026-10-23 10:00:00",
};

function list(items: MediaRow[], total = items.length): MediaList {
  return { items, total, limit: 60, offset: 0 };
}

async function render() {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  await act(async () => {
    root!.render(<UnplacedImages sources={[]} />);
  });
  await act(async () => {});
  return container;
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

describe("Unplaced images (R46.12)", () => {
  it("hides when nothing is unplaced or ignored", async () => {
    vi.spyOn(api, "media").mockResolvedValue(list([]));
    const el = await render();
    expect(el.textContent).toBe("");
  });

  it("shows the badge and saves a person's caption + target", async () => {
    vi.spyOn(api, "media").mockImplementation(async (p) => (p?.attribution === "ignored" ? list([]) : list([row], 3)));
    vi.spyOn(api, "listItems").mockResolvedValue({ items: [{ id: 9, source_id: 1, title: "Guide", path: "guide.md", is_pii: false }], total: 1 });
    const patch = vi.spyOn(api, "patchMedia").mockResolvedValue({ ...row, attribution: "manual" });
    const el = await render();
    expect(el.querySelector('[aria-label="3 unplaced"]')?.textContent).toBe("3");
    expect(el.textContent).toContain("Nothing is suggested automatically");

    const textarea = el.querySelector("textarea")!;
    const select = el.querySelector("select")!;
    await act(async () => {
      select.dispatchEvent(new FocusEvent("focus"));
      select.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
    });
    await act(async () => {});
    await act(async () => {
      const setText = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!;
      setText.call(textarea, "  Payout flow  ");
      textarea.dispatchEvent(new Event("input", { bubbles: true }));
      const setSel = Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value")!.set!;
      setSel.call(select, "9");
      select.dispatchEvent(new Event("change", { bubbles: true }));
    });
    const save = [...el.querySelectorAll("button")].find((b) => b.textContent === "Save")!;
    await act(async () => {
      save.click();
    });
    expect(patch).toHaveBeenCalledWith("a1", { caption: "Payout flow", target: { item_id: 9 } });
  });
});
