import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { api } from "../api";
import type { Entity } from "../types";
import { RecordsTab } from "./RecordsTab";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let root: Root | null = null;
let container: HTMLDivElement | null = null;

function setSelect(sel: HTMLSelectElement, value: string) {
  sel.value = value;
  sel.dispatchEvent(new Event("change", { bubbles: true }));
}

function setInput(input: HTMLInputElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, "value")!.set!;
  setter.call(input, value);
  input.dispatchEvent(new Event("input", { bubbles: true }));
}

function click(el: HTMLElement) {
  el.dispatchEvent(new MouseEvent("click", { bubbles: true }));
}

function button(c: HTMLElement, text: string): HTMLButtonElement {
  const b = Array.from(c.querySelectorAll("button")).find((x) => x.textContent === text);
  if (!b) throw new Error(`no button "${text}" in: ${c.textContent}`);
  return b as HTMLButtonElement;
}

/** The PATCH/dispute endpoints answer with the full entity row. */
function entityWithStatus(id: number, status: string): Entity {
  return {
    id, item_id: 3, kind: "decision", summary: "", reasoning: "record parser", confidence: 1, author: "", source_ref: "",
    status, owner: "", window_text: "", dispute_count: 1, created_at: "", updated_at: "",
  };
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

async function renderTab() {
  vi.spyOn(api, "listDecisions").mockResolvedValue({
    decisions: [
      {
        entity_id: 1, record_id: "D0005", summary: "Stripe provider", status: "superseded", current: false, owner: "Maya",
        supersedes: [], superseded_by: "D0012", review_status: "unverified", created_at: "",
        provenance: { item_id: 3, source_id: 1, path: "decisions/d5.md", title: "d5", chunk_id: null },
      },
    ],
    total: 1, blocked: 0, refusal: null,
  });
  vi.spyOn(api, "listActions").mockResolvedValue({
    actions: [
      {
        entity_id: 2, task: "draft spec", owner: "Alex", due: "2000-01-02", done: false, review_status: "unverified", created_at: "",
        provenance: { item_id: 4, source_id: 1, path: "meetings/retro.md", title: "retro", chunk_id: 9 },
      },
    ],
    total: 1, blocked: 1, refusal: null,
  });
  vi.spyOn(api, "glossary").mockResolvedValue([]);
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  await act(async () => {
    root!.render(<RecordsTab />);
  });
  await act(async () => {});
  return container;
}

describe("RecordsTab R45.25 (Decisions + Actions + Glossary)", () => {
  it("defaults to Decisions and shows supersession with provenance", async () => {
    const c = await renderTab();
    expect(api.listDecisions).toHaveBeenCalledWith({ status: "current", q: undefined });
    expect(c.textContent).toContain("D0005");
    expect(c.textContent).toContain("Superseded by D0012");
    expect(c.textContent).toContain("decisions/d5.md");
  });

  it("offers no review queue: no All entities view, no Looks right, no confidence", async () => {
    const c = await renderTab();
    const labels = Array.from(c.querySelectorAll("button")).map((b) => b.textContent);
    expect(labels.slice(0, 3)).toEqual(["Decisions", "Actions", "Glossary"]);
    expect(c.textContent).not.toContain("All entities");
    expect(c.textContent).not.toContain("Looks right");
    expect(c.textContent).not.toContain("conf ");
  });

  it("falls back to Decisions when the retired All entities view is remembered", async () => {
    localStorage.setItem("entities-view", "all");
    const c = await renderTab();
    expect(api.listDecisions).toHaveBeenCalled();
    expect(c.textContent).toContain("D0005");
  });

  it("Actions view lists open actions, flags overdue and the gate", async () => {
    const c = await renderTab();
    await act(async () => click(button(c, "Actions")));
    await act(async () => {});
    expect(api.listActions).toHaveBeenCalledWith({ owner: undefined, state: "open", overdue: false });
    expect(c.textContent).toContain("draft spec");
    expect(c.textContent).toContain("(overdue)");
    expect(c.textContent).toContain("hidden because the answer provider");
  });

  it("Glossary view opens the glossary and remembers the choice", async () => {
    const c = await renderTab();
    await act(async () => click(button(c, "Glossary")));
    await act(async () => {});
    expect(api.glossary).toHaveBeenCalled();
    expect(c.querySelector("[aria-expanded]")?.getAttribute("aria-expanded")).toBe("true");
    expect(localStorage.getItem("entities-view")).toBe("glossary");
  });

  it("disputes a decision with a required reason, then restores it", async () => {
    const dispute = vi.spyOn(api, "disputeEntity").mockResolvedValue(entityWithStatus(1, "disputed"));
    const update = vi.spyOn(api, "updateEntity").mockResolvedValue(entityWithStatus(1, "unverified"));
    const c = await renderTab();
    await act(async () => click(button(c, "Dispute…")));
    const record = button(c, "Record dispute");
    expect(record.disabled).toBe(true);
    await act(async () => setInput(c.querySelector("input[placeholder^='Why is this wrong']") as HTMLInputElement, "  wrong vendor  "));
    await act(async () => click(button(c, "Record dispute")));
    await act(async () => {});
    expect(dispute).toHaveBeenCalledWith(1, "wrong vendor");
    expect(c.textContent).toContain("Disputed — hidden from this list and from AI agents until restored.");
    expect(c.textContent).toContain("Stripe provider");

    await act(async () => click(button(c, "Restore")));
    await act(async () => {});
    expect(update).toHaveBeenCalledWith(1, { status: "unverified" });
    expect(c.textContent).not.toContain("Disputed —");
    expect(button(c, "Dispute…")).toBeTruthy();
  });

  it("the Disputed filters ask the API for disputed rows only", async () => {
    const c = await renderTab();
    await act(async () => setSelect(c.querySelector("select[title='Filter by decision state']") as HTMLSelectElement, "disputed"));
    await act(async () => {});
    expect(api.listDecisions).toHaveBeenLastCalledWith({ disputed: true, q: undefined });

    await act(async () => click(button(c, "Actions")));
    await act(async () => {});
    await act(async () => setSelect(c.querySelector("select[title='Filter by state']") as HTMLSelectElement, "disputed"));
    await act(async () => {});
    expect(api.listActions).toHaveBeenLastCalledWith({ owner: undefined, state: "all", disputed: true, overdue: false });
  });
});
