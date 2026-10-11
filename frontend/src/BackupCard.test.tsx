import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { api } from "./api";
import { BackupCard } from "./BackupCard";
import type { BackupImportReport } from "./types";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const REPORT: BackupImportReport = {
  dry_run: true,
  mode: "dry_run",
  format_version: 1,
  app_version: "1.0.19",
  created_at: "2026-10-11T00:00:00+00:00",
  counts: { source: 1, item: 2, entity: 3 },
  imported: { source: 1, item: 2, entity: 3 },
  skipped: {},
  conflicts: [{ type: "entity", uid: "u1", action: "kept-local" }],
  needs_secret: [{ source_uid: "u1", source_name: "payments", field: "token" }],
  folder_paths: [{ source_uid: "u1", name: "payments", path: "C:\\Docs\\payments" }],
  remap_applied: [],
  secrets_imported: 0,
  secrets_note: null,
  safety_backup: null,
  reembed_source_ids: [],
  warnings: [],
};

let root: Root | null = null;
let container: HTMLDivElement | null = null;

async function renderCard() {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  await act(async () => {
    root!.render(<BackupCard />);
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

function buttons(host: HTMLElement, label: string): HTMLButtonElement[] {
  return [...host.querySelectorAll("button")].filter((b) => b.textContent?.includes(label)) as HTMLButtonElement[];
}

function setFiles(host: HTMLElement, file: File) {
  const input = host.querySelector('input[type="file"]') as HTMLInputElement;
  Object.defineProperty(input, "files", { value: [file], configurable: true });
  input.dispatchEvent(new Event("change", { bubbles: true }));
}

async function click(host: HTMLElement, label: string) {
  const button = buttons(host, label)[0];
  await act(async () => {
    button.click();
  });
  await act(async () => {});
}

async function setValue(input: HTMLInputElement, value: string) {
  // React's controlled inputs ignore a plain `.value =` write; call the native
  // setter and dispatch the event React listens to.
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
  await act(async () => {
    setter.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

describe("BackupCard", () => {
  it("exports with the chosen options and triggers a download", async () => {
    const exportSpy = vi
      .spyOn(api, "backupExport")
      .mockResolvedValue({ blob: new Blob(["zip-bytes"]), filename: "ac.anchorcore-backup" });
    const objectUrl = vi.fn(() => "blob:test");
    const revoke = vi.fn();
    Object.defineProperty(URL, "createObjectURL", { value: objectUrl, configurable: true });
    Object.defineProperty(URL, "revokeObjectURL", { value: revoke, configurable: true });
    // jsdom has no navigation; the anchor click is the download trigger.
    const anchorClick = vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(() => {});

    const host = await renderCard();
    const [secrets] = [...host.querySelectorAll('input[type="checkbox"]')] as HTMLInputElement[];
    await act(async () => {
      secrets.click();
    });
    // no passphrase yet → export stays disabled
    expect(buttons(host, "Export archive")[0].disabled).toBe(true);

    const pass = host.querySelector('input[type="password"]') as HTMLInputElement;
    await setValue(pass, "correct horse");
    await click(host, "Export archive");

    expect(exportSpy).toHaveBeenCalledWith(
      expect.objectContaining({ include_secrets: true, passphrase: "correct horse" })
    );
    expect(anchorClick).toHaveBeenCalled();
    expect(revoke).toHaveBeenCalled();
    expect(host.textContent).toContain("Exported ac.anchorcore-backup");
  });

  it("shows the dry-run report with credentials and folder remap before importing", async () => {
    const importSpy = vi.spyOn(api, "backupImport").mockResolvedValue(REPORT);
    const host = await renderCard();
    setFiles(host, new File(["zip"], "ac.anchorcore-backup"));
    await click(host, "Preview (dry run)");

    expect(importSpy).toHaveBeenCalledWith(expect.any(File), expect.objectContaining({ dry_run: true, mode: "merge" }));
    expect(host.textContent).toContain("source 1");
    expect(host.textContent).toContain("payments — token");
    expect(host.textContent).toContain("1 conflict(s)");
    const remapInput = [...host.querySelectorAll("input")].find(
      (i) => (i as HTMLInputElement).value === "C:\\Docs\\payments"
    ) as HTMLInputElement;
    expect(remapInput).toBeTruthy();

    // a real merge reuses the (pre-filled) remap and reports what changed
    importSpy.mockResolvedValue({ ...REPORT, dry_run: false, mode: "merge", warnings: [] });
    await click(host, "Import (merge)");
    expect(importSpy).toHaveBeenLastCalledWith(
      expect.any(File),
      expect.objectContaining({ dry_run: false, mode: "merge", remap: { u1: "C:\\Docs\\payments" } })
    );
    expect(host.textContent).toContain("Imported 6 record(s)");
  });

  it("requires the typed confirmation before replace", async () => {
    const importSpy = vi.spyOn(api, "backupImport").mockResolvedValue({ ...REPORT, dry_run: false, mode: "replace" });
    const host = await renderCard();
    setFiles(host, new File(["zip"], "ac.anchorcore-backup"));

    expect(buttons(host, "Replace all data")[0].disabled).toBe(true);
    const confirmInput = host.querySelector('input[placeholder="replace"]') as HTMLInputElement;
    await setValue(confirmInput, "replace");
    expect(buttons(host, "Replace all data")[0].disabled).toBe(false);

    await click(host, "Replace all data");
    expect(importSpy).toHaveBeenCalledWith(
      expect.any(File),
      expect.objectContaining({ mode: "replace", dry_run: false, confirm: "replace" })
    );
  });
});
