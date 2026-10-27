import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { Source } from "../../types";
import { LABEL_OPTIONS } from "./labels";
import { SourceRow } from "./SourceRow";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let root: Root | null = null;
let container: HTMLDivElement | null = null;

function mkSource(label: Source["label"]): Source {
  return {
    id: 1,
    connector: "folder",
    name: "docs",
    enabled: true,
    label,
    last_synced_at: null,
    last_error: null,
    error_count: 0,
    created_at: "2026-10-13T00:00:00+00:00",
  };
}

async function renderRow(label: Source["label"]) {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  await act(async () => {
    // R30.1: SourceRow refreshes the jobs query after a sync/stop, so it needs a client.
    root!.render(
      <QueryClientProvider client={new QueryClient()}>
        <SourceRow source={mkSource(label)} config={{ path: "/tmp/docs" }} jobs={[]} onChanged={() => {}} />
      </QueryClientProvider>,
    );
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

describe("source labels (R35.4: pii bundled into sensitive)", () => {
  it("offers only internal/public/sensitive — no pii option", () => {
    expect(LABEL_OPTIONS.map((o) => o.value)).toEqual(["internal", "public", "sensitive"]);
  });

  it("renders a sensitive source with its hint", async () => {
    const host = await renderRow("sensitive");
    expect(host.textContent).toContain("label: sensitive");
    expect(host.textContent).toContain("local models only");
  });
});
