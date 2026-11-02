import { describe, expect, it } from "vitest";
import { onlySampleSources, startersFromRecords } from "./starters";
import type { ActionRecord, DecisionRecord } from "./types";

const prov = {} as DecisionRecord["provenance"];
const decision = (summary: string, created_at: string, current = true): DecisionRecord => ({
  entity_id: 1, record_id: "", summary, status: "accepted", current, owner: "", supersedes: [], superseded_by: "",
  review_status: "unverified", created_at, provenance: prov,
});
const action = (owner: string, created_at: string, done = false): ActionRecord => ({
  entity_id: 2, task: "t", owner, due: "", done, review_status: "unverified", created_at, provenance: prov,
});

describe("onlySampleSources", () => {
  it("is true only when every source is the sample folder", () => {
    expect(onlySampleSources(["/app/sample/"], 1)).toBe(true);
    expect(onlySampleSources(["C:\\AnchorCore\\sample"], 1)).toBe(true);
    expect(onlySampleSources(["/app/sample", "/me/notes"], 2)).toBe(false);
    expect(onlySampleSources(["/app/sample"], 2)).toBe(false); // a non-folder source too
    expect(onlySampleSources([], 0)).toBe(false);
    expect(onlySampleSources(["/me/sample-ai"], 1)).toBe(false);
  });
});

describe("startersFromRecords", () => {
  it("uses the newest current decision and the newest open action's owner", () => {
    const qs = startersFromRecords(
      [decision("Old topic", "2026-01-01"), decision("Decision: Ledger v2.", "2026-03-01"), decision("Gone", "2026-04-01", false)],
      [action("Dana", "2026-02-01"), action("Sam", "2026-05-01", true)],
    );
    expect(qs).toEqual(["What was decided about Ledger v2, and why?", "What does Dana own?"]);
  });

  it("skips long summaries and blank owners, and is empty with no records", () => {
    expect(startersFromRecords([decision("x".repeat(90), "2026-01-01")], [action("  ", "2026-01-01")])).toEqual([]);
    expect(startersFromRecords([], [])).toEqual([]);
  });
});
