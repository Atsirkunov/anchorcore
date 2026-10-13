import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { api } from "../api";
import type { Entity } from "../types";
import { EntitiesTab } from "./EntitiesTab";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let root: Root | null = null;
let container: HTMLDivElement | null = null;

const ENT: Entity = {
  id: 7,
  item_id: 3,
  kind: "decision",
  summary: "Adopt SQLite for local storage",
  reasoning: "Single file, zero ops.",
  confidence: 0.8,
  author: "ao",
  source_ref: "notes.md",
  status: "unverified",
  owner: "",
  window_text: "we will use sqlite",
  dispute_count: 0,
  item_is_pii: true, // backend still sends the mark; the UI must ignore it
  created_at: "2026-10-13T00:00:00+00:00",
  updated_at: "2026-10-13T00:00:00+00:00",
};

async function renderTab(entities: Entity[]) {
  vi.spyOn(api, "listEntities").mockResolvedValue(entities);
  vi.spyOn(api, "tags").mockResolvedValue([]);
  vi.spyOn(api, "sections").mockResolvedValue([]);
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  await act(async () => {
    root!.render(<EntitiesTab />);
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

describe("EntitiesTab (PII deprecated 2026-10-13)", () => {
  it("shows no PII affordance even when the backend sends the mark", async () => {
    const host = await renderTab([ENT]);
    expect(host.textContent).toContain("Adopt SQLite");
    expect(host.textContent).not.toMatch(/PII/i);
    const buttons = [...host.querySelectorAll("button")].map((b) => b.textContent ?? "");
    expect(buttons.join(" ")).not.toMatch(/PII/i);
  });

  it("shows no PII affordance when empty", async () => {
    const host = await renderTab([]);
    expect(host.textContent).not.toMatch(/PII/i);
  });
});
