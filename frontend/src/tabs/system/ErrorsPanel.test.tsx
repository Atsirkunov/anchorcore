import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { api } from "../../api";
import type { EventGroup, EventSummary } from "../../types";
import { ErrorsPanel } from "./ErrorsPanel";
import { filterMatch, groupMatch } from "./errorsMatch";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let root: Root | null = null;
let container: HTMLDivElement | null = null;

const failed: EventGroup = {
  id: 9,
  component: "pipeline",
  level: "error",
  source_id: null,
  source_name: null,
  message: "extract failed",
  detail: "x".repeat(300),
  count: 3,
  first_seen: "2026-10-01 10:00:00",
  last_seen: "2026-10-03 10:00:00",
  dismissed: false,
};

const summary: EventSummary = {
  levels: { error: 3, warning: 1, info: 12 },
  components: [{ component: "mcp", count: 12 }, { component: "pipeline", count: 3 }],
  sources: [],
  dismissed: 0,
};

async function render() {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  await act(async () => {
    root!.render(<ErrorsPanel />);
  });
  return container;
}

function button(el: HTMLElement, text: string): HTMLButtonElement {
  const b = [...el.querySelectorAll("button")].find((x) => (x.textContent ?? "").startsWith(text));
  if (!b) throw new Error(`no button ${text}`);
  return b;
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

describe("errors & warnings panel (R47.2)", () => {
  it("matches target exactly the row or the active filters", () => {
    expect(groupMatch(failed)).toEqual({ component: "pipeline", levels: ["error"], source_id: null, message: "extract failed" });
    expect(filterMatch(["error", "warning"], "", null)).toEqual({ levels: ["error", "warning"] });
    expect(filterMatch(["error"], "pipeline", 4)).toEqual({ levels: ["error"], component: "pipeline", source_id: 4 });
  });

  it("starts on errors + warnings, folds repeats and expands long detail", async () => {
    const groups = vi.spyOn(api, "systemErrorGroups").mockResolvedValue([failed]);
    vi.spyOn(api, "systemErrorSummary").mockResolvedValue(summary);
    const el = await render();

    expect(groups.mock.calls[0][0]).toMatchObject({ levels: ["error", "warning"], includeDismissed: false });
    expect(button(el, "info").getAttribute("aria-pressed")).toBe("false");
    expect(el.textContent).toContain("×3");
    expect(el.textContent).not.toContain("x".repeat(250));
    await act(async () => {
      button(el, "More").click();
    });
    expect(el.textContent).toContain("x".repeat(300));
  });

  it("dismisses one row by its own key and clears the shown filter", async () => {
    vi.spyOn(api, "systemErrorGroups").mockResolvedValue([failed]);
    vi.spyOn(api, "systemErrorSummary").mockResolvedValue(summary);
    const dismiss = vi.spyOn(api, "dismissErrors").mockResolvedValue({ dismissed: 3 });
    const el = await render();

    await act(async () => {
      button(el, "Dismiss").click();
    });
    expect(dismiss).toHaveBeenLastCalledWith(groupMatch(failed));
    expect(el.textContent).toContain("Dismissed 3.");

    await act(async () => {
      button(el, "Clear shown").click();
    });
    expect(dismiss).toHaveBeenLastCalledWith({ levels: ["error", "warning"] });
    expect(el.textContent).toContain("Cleared 3.");
  });
});
