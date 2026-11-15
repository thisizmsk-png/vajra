// The percentile the real-browser typing-lag gate (scripts/typing-lag.mjs) compares with its 50 ms budget.
import { describe, expect, it } from "vitest";
// @ts-expect-error a plain .mjs module, no types
import { pct } from "../scripts/pct.mjs";

describe("typing-lag script", () => {
  it("picks the nearest-rank percentile, and null for no samples", () => {
    const xs = Array.from({ length: 100 }, (_, i) => i + 1);
    expect(pct(xs, 95)).toBe(95);
    expect(pct(xs, 50)).toBe(50);
    expect(pct([3, 1, 2], 100)).toBe(3);
    expect(pct([], 95)).toBeNull();
  });
});
