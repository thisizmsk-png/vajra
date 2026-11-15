import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { parseWorkerProvider, ranOn, workerLine } from "../src/workerProvider";

// The bytes the core serialises for a worker's `provider` (checked byte for byte by
// app/core/src/providers/effective_tests.rs): the UI parser must read them, not drop them to "no provider".
const golden = JSON.parse(readFileSync(resolve(__dirname, "../../core/tests/fixtures/launch/worker-provider.json"), "utf8"));

describe("the core's worker provider object", () => {
  it("parses to a provider with its label, and the matching request raises no 'ran on'", () => {
    const p = parseWorkerProvider(golden);
    expect(p).not.toBeNull();
    expect([p?.account, p?.label, p?.family, p?.model, p?.billing, p?.error]).toEqual(["openrouter", "OpenRouter", "vendor", "vendor/model-a", "gateway", null]);
    expect(p?.requested?.model).toBe("vendor/model-a");
    expect(p && workerLine(p)).toBe("vendor/model-a via OpenRouter");
    expect(p && ranOn(p)).toBeNull();
  });
  it("shows 'ran on' when the effective model differs from the request", () => {
    const p = parseWorkerProvider({ ...golden, model: "vendor/model-b" });
    expect(p && ranOn(p)).not.toBeNull();
  });
  it("reads a model the harness did not report as such", () => {
    const p = parseWorkerProvider({ ...golden, model: "unknown" });
    expect(p?.model).toBe("unknown");
  });
});
