import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { api } from "../../api";
import { cancelledLine, jobStopLabel } from "../../jobControlsText";
import type { CancelJobResult, Job, Source } from "../../types";
import { SourceRow } from "./SourceRow";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let root: Root | null = null;
let container: HTMLDivElement | null = null;

const source: Source = {
  id: 3,
  connector: "folder",
  name: "My folder",
  enabled: true,
  label: "internal",
  last_synced_at: null,
  last_error: null,
  error_count: 0,
  created_at: "2026-10-12T00:00:00+00:00",
};

function mkJob(id: number, kind: Job["kind"], status: Job["status"], processed: number, total: number): Job {
  return { id, source_id: 3, kind, status, total, processed, result: {}, error: null, created_at: "2026-10-12 10:00:00", started_at: null, finished_at: null };
}

async function renderRow(jobs: Job[]) {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  await act(async () => {
    root!.render(
      <QueryClientProvider client={new QueryClient()}>
        <SourceRow source={source} config={{ path: "/tmp/docs" }} jobs={jobs} onChanged={() => {}} />
      </QueryClientProvider>,
    );
  });
  return container;
}

function stopButtons(el: HTMLElement): HTMLButtonElement[] {
  return [...el.querySelectorAll("button")].filter((b) => (b.textContent ?? "").startsWith("Stop"));
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

describe("per-job Stop controls (R30.4)", () => {
  it("labels helpers name kind + id and the kept count", () => {
    expect(jobStopLabel({ id: 11, kind: "reclassify" })).toBe("Stop reclassify #11");
    expect(cancelledLine(37, 513)).toBe("cancelled — 37/513 docs kept");
  });

  it("renders one labeled control per active job and stops exactly the one clicked", async () => {
    let resolve: (r: CancelJobResult) => void = () => {};
    const cancel = vi.spyOn(api, "cancelJob").mockImplementation(() => new Promise((r) => (resolve = r)));
    const el = await renderRow([mkJob(10, "sync", "running", 200, 513), mkJob(11, "embed", "pending", 0, 0)]);

    const buttons = stopButtons(el);
    expect(buttons.map((b) => b.textContent)).toEqual(["Stop sync #10", "Stop embed #11"]);
    expect(buttons[0].getAttribute("aria-label")).toContain('on "My folder"');

    await act(async () => {
      buttons[1].click();
    });
    expect(cancel).toHaveBeenCalledTimes(1);
    expect(cancel).toHaveBeenCalledWith(11);
    // in flight: that control disables, the other stays live
    const during = [...el.querySelectorAll("button")].find((b) => b.textContent === "Stopping #11…")!;
    expect(during.disabled).toBe(true);
    expect(stopButtons(el)[0].disabled).toBe(false);

    await act(async () => {
      resolve({ cancelled: true, job_id: 11, kind: "embed", was: "pending", status: "cancelled", processed: 0, total: 0 });
    });
    expect(el.textContent).toContain("#11 embed cancelled before it started");
  });

  it("shows the kept count after stopping a running job, and errors inline", async () => {
    const cancel = vi
      .spyOn(api, "cancelJob")
      .mockResolvedValueOnce({ cancelled: true, job_id: 10, kind: "sync", was: "running", status: "cancelled", processed: 37, total: 513 })
      .mockRejectedValueOnce(new Error("Job #12 is done, not running"));
    const el = await renderRow([mkJob(10, "sync", "running", 37, 513), mkJob(12, "embed", "running", 5, 9)]);
    const [first, second] = stopButtons(el);
    await act(async () => {
      first.click();
    });
    await act(async () => {
      second.click();
    });
    expect(cancel).toHaveBeenCalledTimes(2);
    expect(el.textContent).toContain("#10 sync cancelled — 37/513 docs kept");
    expect(el.textContent).toContain("Stop embed #12 failed: Job #12 is done, not running");
  });
});
