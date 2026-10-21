import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { api } from "../api";
import type { Entity, MergeProposal, VocabProposal } from "../types";
import { AllEntitiesView as EntitiesTab, EntitiesTab as EntitiesTabShell } from "./EntitiesTab";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let root: Root | null = null;
let container: HTMLDivElement | null = null;

function mkEntity(over: Partial<Entity> & { id: number }): Entity {
  return {
    item_id: 3,
    kind: "note",
    summary: `Summary ${over.id}`,
    reasoning: "",
    confidence: 0.8,
    author: "ao",
    source_ref: "notes.md",
    status: "unverified",
    owner: "",
    window_text: "window text",
    dispute_count: 0,
    created_at: "2026-10-13T00:00:00+00:00",
    updated_at: "2026-10-13T00:00:00+00:00",
    ...over,
  };
}

const ENT = mkEntity({ id: 7, kind: "decision", summary: "Adopt SQLite for local storage", reasoning: "Single file, zero ops.", item_is_pii: true });

async function renderTab(entities: Entity[], dupes: MergeProposal[] = [], grouped = true, vocab: VocabProposal[] = []) {
  localStorage.setItem("entities-grouped", grouped ? "1" : "0");
  vi.spyOn(api, "listEntities").mockResolvedValue(entities);
  vi.spyOn(api, "duplicates").mockResolvedValue(dupes);
  vi.spyOn(api, "vocabulary").mockResolvedValue(vocab);
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

describe("EntitiesTab R31.2 (A + Review fold)", () => {
  it("shows every item on first screen instead of an empty needs-review default", async () => {
    const host = await renderTab([
      mkEntity({ id: 1, status: "verified" }),
      mkEntity({ id: 2, status: "unverified" }),
    ]);
    expect(host.textContent).toContain("Summary 1");
    expect(host.textContent).toContain("Summary 2");
    expect(host.textContent).toContain("2 / 2 items");
    expect(host.textContent).toContain("1 need a look");
    // dead + lying controls are gone
    expect(host.textContent).not.toContain("avg ");
    expect(host.textContent).not.toContain("Verify all & collapse");
    expect(host.textContent).not.toContain("Dispute doc");
    expect(host.textContent).not.toContain("✓ Verified");
    const options = [...host.querySelectorAll("select")[1].querySelectorAll("option")].map((o) => o.textContent ?? "");
    expect(options).not.toContain("Stale");
  });

  it("shows visible #ids and honest trust labels", async () => {
    const host = await renderTab([
      mkEntity({ id: 7, kind: "decision", status: "unverified" }),
      mkEntity({ id: 8, kind: "action", status: "verified" }),
      mkEntity({ id: 9, kind: "note", status: "disputed", dispute_count: 2 }),
    ]);
    expect(host.textContent).toContain("#7");
    expect(host.textContent).toContain("Decision");
    expect(host.textContent).toContain("To-do");
    expect(host.textContent).toContain("New");
    expect(host.textContent).toContain("In answers");
    expect(host.textContent).toContain("Disputed");
    expect(host.textContent).toContain("⚑ disputed ×2");
    expect(host.textContent).not.toContain("Unverified");
  });

  it("renders any doc_type bucket and any kind without assuming a closed set", async () => {
    const host = await renderTab([
      mkEntity({ id: 1, kind: "decision", doc_type: "record" }),
      mkEntity({ id: 2, kind: "note", doc_type: "reference" }),
      mkEntity({ id: 3, kind: "note", doc_type: "runbook" }),
      mkEntity({ id: 4, kind: "risk", doc_type: "decision_log" }),
    ]);
    for (const label of ["Record", "Reference", "Runbook", "Decision log", "Risk"]) {
      expect(host.textContent).toContain(label);
    }
  });

  it("tells the truth when a filter matches nothing, with a way back", async () => {
    const host = await renderTab([mkEntity({ id: 1, status: "verified" })]);
    const statusSel = host.querySelectorAll("select")[1] as HTMLSelectElement;
    await act(async () => {
      setSelect(statusSel, "needs_review");
    });
    await act(async () => {});
    expect(host.textContent).toContain("Nothing here — 1 items total, none need a look.");
    expect(host.textContent).not.toContain("No entities yet");
    const back = [...host.querySelectorAll("button")].find((b) => b.textContent === "Show all items");
    expect(back).toBeDefined();
    await act(async () => {
      click(back!);
    });
    await act(async () => {});
    expect(host.textContent).toContain("Summary 1");
  });

  it("folds duplicate proposals inline with both summaries and working decisions", async () => {
    const a = mkEntity({ id: 11, kind: "note", summary: "Billing runs on the old provider" });
    const b = mkEntity({ id: 12, kind: "note", summary: "Billing still uses the legacy provider" });
    const decide = vi.spyOn(api, "decideMerge").mockResolvedValue({ merged: true, entity_id: 11 });
    const host = await renderTab([a, b], [{ id: 5, entity_a_id: 11, entity_b_id: 12, reason: "similar summaries" }]);
    expect(host.textContent).toContain("Possible doubles (1)");
    expect(host.textContent).toContain("Billing runs on the old provider");
    expect(host.textContent).toContain("Billing still uses the legacy provider");
    const merge = [...host.querySelectorAll("button")].find((btn) => btn.textContent === "Merge — keeps #11");
    expect(merge).toBeDefined();
    await act(async () => {
      click(merge!);
    });
    await act(async () => {});
    expect(decide).toHaveBeenCalledWith(5, "merge");
  });

  it("folds vocabulary proposals inline with working approve/reject", async () => {
    const decide = vi.spyOn(api, "decideVocab").mockResolvedValue({ approved: true, term: "dunning", expansion: "payment reminder" });
    const host = await renderTab([ENT], [], true, [
      { id: 9, term: "dunning", expansion: "payment reminder", origin: "llm", confirmations: 1, item_id: 3, item_title: "Alpha doc", external_id: "a.md" },
    ]);
    expect(host.textContent).toContain("Words to confirm (1)");
    expect(host.textContent).toContain("“dunning” means “payment reminder”");
    expect(host.textContent).toContain("Alpha doc");
    const confirm = [...host.querySelectorAll("button")].find((btn) => btn.textContent === "Confirm");
    expect(confirm).toBeDefined();
    await act(async () => {
      click(confirm!);
    });
    await act(async () => {});
    expect(decide).toHaveBeenCalledWith(9, "approve");
  });

  it("Dispute records through the audit endpoint instead of a silent PATCH", async () => {
    const dispute = vi.spyOn(api, "disputeEntity").mockResolvedValue(mkEntity({ id: 7, status: "disputed" }));
    const patch = vi.spyOn(api, "updateEntity").mockResolvedValue(mkEntity({ id: 7, status: "verified" }));
    const host = await renderTab([ENT]);
    const disputeBtn = [...host.querySelectorAll("button")].find((btn) => btn.textContent === "Dispute…");
    expect(disputeBtn).toBeDefined();
    await act(async () => {
      click(disputeBtn!);
    });
    const input = host.querySelector('input[placeholder^="Why is"]') as HTMLInputElement;
    expect(input).not.toBeNull();
    await act(async () => {
      setInput(input, "outdated after the migration");
    });
    const record = [...host.querySelectorAll("button")].find((btn) => btn.textContent === "Record dispute");
    await act(async () => {
      click(record!);
    });
    await act(async () => {});
    expect(dispute).toHaveBeenCalledWith(7, "outdated after the migration");
    expect(patch).not.toHaveBeenCalled();
  });

  it("Looks right keeps the row in answers via PATCH", async () => {
    const patch = vi.spyOn(api, "updateEntity").mockResolvedValue(mkEntity({ id: 7, status: "verified" }));
    const host = await renderTab([ENT]);
    const ok = [...host.querySelectorAll("button")].find((btn) => btn.textContent === "Looks right");
    await act(async () => {
      click(ok!);
    });
    await act(async () => {});
    expect(patch).toHaveBeenCalledWith(7, { status: "verified" });
  });

  it("makes recorded disputes readable per row", async () => {
    vi.spyOn(api, "entityDisputes").mockResolvedValue([
      { id: 3, entity_id: 7, reason: "stale number", user: "ao", created_at: "2026-10-14T10:00:00+00:00" },
    ]);
    const host = await renderTab([mkEntity({ id: 7, status: "disputed", dispute_count: 1 })]);
    const history = [...host.querySelectorAll("button")].find((btn) => btn.textContent?.startsWith("History"));
    await act(async () => {
      click(history!);
    });
    await act(async () => {});
    expect(host.textContent).toContain("stale number");
  });

  it("flat view keeps working with refs and the same trust actions", async () => {
    const host = await renderTab([mkEntity({ id: 7, source_ref: "meetings/m1.md" })], [], false);
    expect(host.textContent).toContain("meetings/m1.md");
    expect(host.textContent).toContain("Looks right");
    expect(host.textContent).toContain("Dispute…");
  });

  it("conflict rows show the other item's summary, not just its id", async () => {
    vi.spyOn(api, "listConflicts").mockResolvedValue([{ other_entity_id: 9, note: "dates differ", created_at: "2026-10-14T00:00:00+00:00" }]);
    const host = await renderTab([mkEntity({ id: 7 }), mkEntity({ id: 9, summary: "Launch is in March" })]);
    const toggle = [...host.querySelectorAll("button")].find((btn) => btn.textContent?.startsWith("Conflicts"));
    await act(async () => {
      click(toggle!);
    });
    await act(async () => {});
    expect(host.textContent).toContain("contradicts #9");
    expect(host.textContent).toContain("Launch is in March");
  });
});

describe("EntitiesTab R45.9 (Decisions + Actions views)", () => {
  async function renderShell() {
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
    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);
    await act(async () => {
      root!.render(<EntitiesTabShell />);
    });
    await act(async () => {});
    return container;
  }

  it("defaults to Decisions and shows supersession with provenance", async () => {
    const c = await renderShell();
    expect(api.listDecisions).toHaveBeenCalledWith({ status: "current", q: undefined });
    expect(c.textContent).toContain("D0005");
    expect(c.textContent).toContain("Superseded by D0012");
    expect(c.textContent).toContain("decisions/d5.md");
  });

  it("Actions view lists open actions, flags overdue and the gate", async () => {
    const c = await renderShell();
    const btn = Array.from(c.querySelectorAll("button")).find((b) => b.textContent === "Actions")!;
    await act(async () => {
      click(btn);
    });
    await act(async () => {});
    expect(api.listActions).toHaveBeenCalledWith({ owner: undefined, state: "open", overdue: false });
    expect(c.textContent).toContain("draft spec");
    expect(c.textContent).toContain("(overdue)");
    expect(c.textContent).toContain("hidden because the answer provider");
  });
});
