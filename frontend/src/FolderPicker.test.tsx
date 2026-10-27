import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { api } from "./api";
import { FolderPicker } from "./FolderPicker";
import type { FolderListing } from "./types";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

function listing(over: Partial<FolderListing> = {}): FolderListing {
  return { path: null, parent: null, home: "/home/u", dirs: [], truncated: false, ...over };
}

const ROOTS = listing({ dirs: [{ name: "Home", path: "/home/u" }] });
const HOME = listing({
  path: "/home/u",
  parent: "/home",
  home: "/home/u",
  dirs: [
    { name: "docs", path: "/home/u/docs" },
    { name: "src", path: "/home/u/src" },
  ],
});

let root: Root | null = null;
let container: HTMLDivElement | null = null;

async function renderPicker(props: { initialPath?: string; onSelect?: (p: string) => void; onCancel?: () => void }) {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  await act(async () => {
    root!.render(
      <FolderPicker initialPath={props.initialPath} onSelect={props.onSelect ?? (() => {})} onCancel={props.onCancel ?? (() => {})} />,
    );
  });
  // flush the effect's async browse
  await act(async () => {});
  return container;
}

function buttons(host: HTMLElement): HTMLButtonElement[] {
  return Array.from(host.querySelectorAll("button"));
}

function click(el: HTMLElement) {
  act(() => {
    el.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
}

async function clickAsync(el: HTMLElement) {
  await act(async () => {
    el.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
}

function setInputValue(input: HTMLInputElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, "value")!.set!;
  act(() => {
    setter.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

function pressKey(el: HTMLElement, key: string) {
  act(() => {
    el.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true }));
  });
}

async function pressKeyAsync(el: HTMLElement, key: string) {
  await act(async () => {
    el.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true }));
  });
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

describe("FolderPicker", () => {
  it("opens the shortcut view when no initial path is given", async () => {
    const browse = vi.spyOn(api, "browseFolders").mockResolvedValue(ROOTS);
    const host = await renderPicker({});
    expect(browse).toHaveBeenCalledWith();
    expect(host.textContent).toContain("Pick a drive or Home to start");
    expect(host.textContent).toContain("Home");
  });

  it("starts at initialPath and falls back to shortcuts with a note when it fails", async () => {
    const browse = vi
      .spyOn(api, "browseFolders")
      .mockRejectedValueOnce(new Error("nope"))
      .mockResolvedValueOnce(ROOTS);
    const host = await renderPicker({ initialPath: "/bad/dir" });
    expect(browse).toHaveBeenNthCalledWith(1, "/bad/dir");
    expect(browse).toHaveBeenNthCalledWith(2);
    expect(host.textContent).toContain('"/bad/dir" is not a folder');
    expect(host.textContent).toContain("Home");
  });

  it("navigates into a clicked folder and picks it with 'Use this folder'", async () => {
    vi.spyOn(api, "browseFolders").mockImplementation(async (path?: string) => (path ? HOME : ROOTS));
    const onSelect = vi.fn();
    const host = await renderPicker({ onSelect });
    const homeRow = buttons(host).find((b) => b.textContent === "Home/");
    expect(homeRow).toBeDefined();
    await clickAsync(homeRow!);
    expect(host.textContent).toContain("Current: /home/u");
    expect(host.textContent).toContain("docs");
    const use = buttons(host).find((b) => b.textContent === "Use this folder");
    click(use!);
    expect(onSelect).toHaveBeenCalledWith("/home/u");
  });

  it("goes to a typed absolute path on Enter", async () => {
    const browse = vi.spyOn(api, "browseFolders").mockImplementation(async (path?: string) => (path ? HOME : ROOTS));
    const host = await renderPicker({});
    const input = host.querySelector('input[placeholder="Absolute folder path"]') as HTMLInputElement;
    setInputValue(input, "  /home/u  ");
    await pressKeyAsync(input, "Enter");
    expect(browse).toHaveBeenLastCalledWith("/home/u");
    expect(host.textContent).toContain("Current: /home/u");
  });

  it("cancels via the Cancel button and the Escape key", async () => {
    vi.spyOn(api, "browseFolders").mockResolvedValue(HOME);
    const onCancel = vi.fn();
    const host = await renderPicker({ onCancel });
    const cancel = buttons(host).find((b) => b.textContent === "Cancel");
    click(cancel!);
    expect(onCancel).toHaveBeenCalledTimes(1);
    pressKey(host.firstElementChild as HTMLElement, "Escape");
    expect(onCancel).toHaveBeenCalledTimes(2);
  });

  it("keeps truncation off the dir buttons so rows keep their height", async () => {
    // A grid-item <button> with overflow:hidden collapses to padding height in
    // Chromium and clips its own label; the truncation trio must live on the
    // inner span, never on the button.
    vi.spyOn(api, "browseFolders").mockResolvedValue(HOME);
    const host = await renderPicker({});
    const row = buttons(host).find((b) => b.textContent === "docs/");
    expect(row).toBeDefined();
    expect(row!.style.overflow).toBe("");
    expect(row!.style.whiteSpace).toBe("");
    const label = row!.querySelector("span");
    expect(label?.textContent).toBe("docs/");
    expect(label!.style.display).toBe("block");
    expect(label!.style.overflow).toBe("hidden");
    expect(label!.style.textOverflow).toBe("ellipsis");
    expect(label!.style.whiteSpace).toBe("nowrap");
  });

  it("surfaces browse failures instead of hanging on Loading", async () => {
    vi.spyOn(api, "browseFolders").mockRejectedValue(new Error("fs offline"));
    const host = await renderPicker({});
    expect(host.textContent).toContain("fs offline");
    expect(host.textContent).not.toContain("Loading…");
  });
});
