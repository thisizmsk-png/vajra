import { describe, expect, it } from "vitest";
import { LOAD_END_S, LOAD_START_S, TOTAL, delivered, fixtureTransport } from "../src/perf/fixture";

describe("perf fixture", () => {
  it("delivers nothing before the load, all TOTAL lines by LOAD_END_S, and never more", () => {
    expect(delivered(0)).toBe(0);
    expect(delivered(LOAD_START_S)).toBe(0);
    expect(delivered(LOAD_END_S)).toBe(TOTAL);
    expect(delivered(LOAD_END_S + 1000)).toBe(TOTAL);
  });

  it("answers tail with a bounded page and a spill range once the core ring is exceeded", async () => {
    const { transport } = fixtureTransport(() => LOAD_END_S + 1);
    const t = (await transport.call("tail", { afterSeq: 0 })) as { lines: unknown[]; gap: boolean; head_seq: number; spill_end: number | null };
    expect(t.lines.length).toBeLessThanOrEqual(700);
    expect(t.gap).toBe(true);
    expect(t.head_seq).toBe(TOTAL);
    expect(t.spill_end).toBeGreaterThan(0);
  });

  it("is a closed world: unknown commands answer empty, the crew has one worker", async () => {
    const { transport } = fixtureTransport(() => 0);
    expect(await transport.call("anything_else")).toEqual({});
    const c = (await transport.call("crew")) as { children: { children: { id: string }[] }[] };
    expect(c.children[0].children[0].id).toBe("agent:a1");
  });
});
