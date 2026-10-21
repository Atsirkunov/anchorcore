import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { api } from "../api";
import { GlossaryPanel } from "./GlossaryPanel";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let root: Root | null = null;
let container: HTMLDivElement | null = null;

afterEach(() => {
  act(() => root?.unmount());
  container?.remove();
  root = null;
  container = null;
  vi.restoreAllMocks();
});

function setInput(input: HTMLInputElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, "value")!.set!;
  setter.call(input, value);
  input.dispatchEvent(new Event("input", { bubbles: true }));
}

function button(host: HTMLElement, text: string) {
  const b = [...host.querySelectorAll("button")].find((x) => x.textContent?.startsWith(text));
  expect(b, `button ${text}`).toBeDefined();
  return b!;
}

async function render() {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  await act(async () => root!.render(<GlossaryPanel />));
  await act(async () => {});
  return container;
}

describe("GlossaryPanel", () => {
  it("adds and removes manual entries", async () => {
    const list = vi
      .spyOn(api, "glossary")
      .mockResolvedValueOnce([])
      .mockResolvedValue([{ id: 4, term: "kyc", expansion: "know your customer", origin: "manual", item_id: null, path: null }]);
    const add = vi.spyOn(api, "addGlossaryEntry").mockResolvedValue({ id: 4, term: "kyc", expansion: "know your customer", origin: "manual", item_id: null, path: null });
    const del = vi.spyOn(api, "deleteGlossaryEntry").mockResolvedValue({ deleted: 4 });
    const host = await render();
    expect(host.textContent).toContain("Glossary (0)");
    await act(async () => button(host, "Glossary").click());
    const [termInput, meaningInput] = [...host.querySelectorAll("input")] as HTMLInputElement[];
    await act(async () => {
      setInput(termInput, "KYC");
      setInput(meaningInput, "know your customer");
    });
    await act(async () => button(host, "Add").click());
    await act(async () => {});
    expect(add).toHaveBeenCalledWith("KYC", "know your customer");
    expect(list).toHaveBeenCalledTimes(2);
    expect(host.textContent).toContain("kyc = know your customer");
    await act(async () => button(host, "Remove").click());
    await act(async () => {});
    expect(del).toHaveBeenCalledWith(4);
  });

  it("marks a found file as a glossary", async () => {
    vi.spyOn(api, "glossary").mockResolvedValue([]);
    vi.spyOn(api, "listItems").mockResolvedValue({ items: [{ id: 9, source_id: 1, title: "terms.csv", path: "docs/terms.csv", is_pii: false, is_glossary: false }], total: 1 });
    const mark = vi.spyOn(api, "setItemGlossary").mockResolvedValue({ id: 9, is_glossary: true });
    const host = await render();
    await act(async () => button(host, "Glossary").click());
    const pathInput = [...host.querySelectorAll("input")][2] as HTMLInputElement;
    await act(async () => setInput(pathInput, "docs/terms"));
    await act(async () => button(host, "Find").click());
    await act(async () => {});
    await act(async () => button(host, "Use as glossary").click());
    await act(async () => {});
    expect(mark).toHaveBeenCalledWith(9, true);
    expect(host.textContent).toContain("Stop using as glossary");
  });
});
