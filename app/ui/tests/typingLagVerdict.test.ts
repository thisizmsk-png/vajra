// The typing-lag gate checks what streaming adds over the idle control, not an absolute figure the runner's own rendering decides.
import { describe, expect, it } from "vitest";
// @ts-expect-error a plain .mjs module, no types
import { BUDGET_DELTA_P95_MS, verdict } from "../scripts/verdict.mjs";

const s = (p95_ms: number, extra = {}) => ({ keystrokes: 120, painted: 120, typed: 120, p95_ms, ...extra });

describe("typing-lag verdict", () => {
  it("passes when streaming adds at most the budget over the idle control, however slow the runner is", () => {
    expect(BUDGET_DELTA_P95_MS).toBe(20);
    expect(verdict(s(88.8), { p95_ms: 68.8 }).ok).toBe(true); // slow VM, small delta: fine
    expect(verdict(s(68.8), { p95_ms: 68.8 }).delta_p95_ms).toBe(0);
  });
  it("fails on the CI numbers from 13 Nov (189.2 vs 68.8) and says by how much", () => {
    const v = verdict(s(189.2), { p95_ms: 68.8 });
    expect(v.ok).toBe(false);
    expect(v.delta_p95_ms).toBe(120.4);
    expect(v.reasons[0]).toContain("120.4 ms");
  });
  it("fails when keys went missing, even with a small delta", () => {
    expect(verdict(s(20, { painted: 118 }), { p95_ms: 18 }).ok).toBe(false);
    expect(verdict(s(20, { typed: 119 }), { p95_ms: 18 }).ok).toBe(false);
  });
  it("only applies the absolute 50 ms budget when asked", () => {
    expect(verdict(s(60), { p95_ms: 58 }).ok).toBe(true);
    expect(verdict(s(60), { p95_ms: 58 }, { absolute: true }).ok).toBe(false);
  });
});
