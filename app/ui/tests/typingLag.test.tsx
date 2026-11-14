// The typing-lag gate: p95 at or under 50 ms while 5 agents stream a fake load.
//
// What it measures: the main-thread time one keystroke takes in the Director-General composer, from the input event to
// the committed DOM, while the change feed delivers 5 streams at 20 lines a second each and the followed stream keeps
// growing. jsdom has no paint, so this is the part of keystroke-to-paint that our code owns (handlers plus React commit);
// the real-app number comes from the bench rig (app/bench/ade). A stream render that blocks a keystroke shows up here.
//
// The numbers always print. The budget fails the test only when VAJRA_LAG_GATE=1, which the perf workflow's typing-lag job
// sets, so the general UI suite never fails on a loaded runner. That job is report-only for its first week.
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { setTransport } from "../src/ipc";
import { CurrentProject } from "../src/projects/current";
import type { Current } from "../src/projects/current";
import { DirectorGeneral } from "../src/screens/orch/DirectorGeneral";
import { S } from "../strings";
import { fakeIpc } from "./fakeIpc";

export const BUDGET_P95_MS = 50;
const AGENTS = 5;
const LINES_PER_S = 20;
const KEYSTROKES = 60;
const STREAM = "orchestrator-p_X-g3";

const pct = (xs: number[], p: number) => {
  const s = [...xs].sort((a, b) => a - b);
  return s[Math.min(s.length - 1, Math.ceil((p / 100) * s.length) - 1)];
};

const project = { schema: "vajra.project/1", id: "p_1", name: "api", root: "/src/api", created: null, archived: null, rev: 0 };
const cur: Current = { project, campaigns: [], focus: "c1", runId: null, dgRun: "20261106-2140", proposals: [], };

afterEach(cleanup);

describe("typing lag under load", () => {
  it("pct picks the nearest-rank percentile", () => {
    expect(pct([1, 2, 3, 4, 5, 6, 7, 8, 9, 10], 95)).toBe(10);
    expect(pct([1, 2, 3, 4, 5, 6, 7, 8, 9, 10], 50)).toBe(5);
  });

  it(`keeps p95 at or under ${BUDGET_P95_MS} ms with ${AGENTS} agents streaming`, async () => {
    let n = 3;
    const lines = () => Array.from({ length: n }, (_, i) => ({ seq: i + 1, text: i % 2 ? `lead\tstep ${i} of the plan` : `tool\tEdit file${i}.ts` }));
    const f = fakeIpc({
      lead_status: { available: true, busy: true, name: "Director-General", state: "running", reason: "working", project_id: "p_X", generation: 3, stream: STREAM },
      stream_window: () => ({ lines: lines(), head_seq: n, first_seq: 1, at_start: true, at_end: true, gap: false }),
    });
    setTransport(f.transport);
    try {
      render(<CurrentProject.Provider value={cur}><DirectorGeneral /></CurrentProject.Provider>);
      const box = (await screen.findByRole("textbox", { name: S.dg.ask })) as HTMLTextAreaElement;

      // The fake load: every agent adds LINES_PER_S lines a second, announced on the change feed; the followed stream grows too.
      const feed = setInterval(() => {
        n += AGENTS;
        f.emit("changed", { streams: [STREAM, ...Array.from({ length: AGENTS }, (_, i) => `w-a${i}`)], snapshot_needed: false });
      }, Math.floor(1000 / LINES_PER_S));

      const lag: number[] = [];
      let text = "";
      for (let i = 0; i < KEYSTROKES; i++) {
        await new Promise((r) => setTimeout(r, 25));
        text += "x";
        const t0 = performance.now();
        act(() => { fireEvent.change(box, { target: { value: text } }); });
        const dt = performance.now() - t0;
        expect(box.value).toBe(text);
        lag.push(dt);
      }
      clearInterval(feed);

      const p50 = pct(lag, 50), p95 = pct(lag, 95);
      console.log(`typing lag over ${KEYSTROKES} keystrokes with ${AGENTS} agents streaming: p50 ${p50.toFixed(1)} ms, p95 ${p95.toFixed(1)} ms (budget ${BUDGET_P95_MS})`);
      if (process.env.VAJRA_LAG_GATE === "1") expect(p95).toBeLessThanOrEqual(BUDGET_P95_MS);
    } finally {
      setTransport();
    }
  }, 30_000);
});
