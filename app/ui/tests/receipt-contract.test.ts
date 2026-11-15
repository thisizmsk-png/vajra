import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { newActionId, type Receipt } from "../src/ipc";

const load = (n: string): Receipt => JSON.parse(readFileSync(`${__dirname}/../../core/tests/contract/${n}`, "utf8"));
const KEYS = ["action", "action_id", "current_generation", "detail", "expect_generation", "project_id", "reason", "retry", "rev", "schema", "state", "target", "ts_ms"];

describe("vajra.receipt/1 contract (shared fixtures with the core)", () => {
  it.each(["receipt_done.json", "receipt_stale.json"])("%s has exactly the Receipt fields", (f) => {
    const r = load(f);
    expect(Object.keys(r).sort()).toEqual(KEYS);
    expect(r.schema).toBe("vajra.receipt/1");
    expect(["accepted", "done", "rejected", "failed", "unknown"]).toContain(r.state);
    expect(["no", "safe", "after_refresh"]).toContain(r.retry);
  });

  it("a stale generation says to refresh and names the current generation", () => {
    const r = load("receipt_stale.json");
    expect(r).toMatchObject({ state: "rejected", reason: "stale_generation", retry: "after_refresh", current_generation: 3 });
  });

  it("action ids are valid for the core (8..=64 of A-Z a-z 0-9 - _) and unique", () => {
    const a = newActionId();
    expect(a).toMatch(/^[A-Za-z0-9_-]{8,64}$/);
    expect(newActionId()).not.toBe(a);
  });
});
