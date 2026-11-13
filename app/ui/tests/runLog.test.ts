// W2: the run view follows a core run's log by seq. It asks only for entries after the last one it holds, never
// refetches everything, drops duplicates, stops at a gap and re-asks, and a reload rebuilds the same list.
import { afterEach, describe, expect, it } from "vitest";
import { setTransport, type RunLogEntry } from "../src/ipc";
import { applyPage, emptyRunLog, followRunLog, type RunLogView } from "../src/runLog";
import { fakeIpc } from "./fakeIpc";

afterEach(() => setTransport());

const entry = (seq: number): RunLogEntry => ({ v: 1, seq, prev: `h${seq - 1}`, at_ms: seq, source: "core", kind: "dispatched", body: {}, hash: `h${seq}` });
const page = (seqs: number[], head = Math.max(0, ...seqs)) => ({ run_id: "r", entries: seqs.map(entry), head_seq: head, broken: null, io_error: null });
const seqs = (v: RunLogView) => v.entries.map((e) => e.seq);
const tick = () => new Promise((r) => setTimeout(r, 0));

/** A core log of `n` entries that answers `run_log_since` like the core (after N, at most `cap`). */
function core(n: number, cap = 500) {
  const log = { n };
  const ipc = fakeIpc({
    run_log_since: (a?: Record<string, unknown>) => {
      const after = Number(a?.afterSeq ?? 0);
      const s: number[] = [];
      for (let i = after + 1; i <= Math.min(log.n, after + cap); i++) s.push(i);
      return page(s, log.n);
    },
  });
  setTransport(ipc.transport);
  return { ipc, log };
}

describe("applyPage", () => {
  it("appends entries that continue the view", () => {
    expect(seqs(applyPage(applyPage(emptyRunLog, page([1, 2])), page([3, 4])))).toEqual([1, 2, 3, 4]);
  });
  it("drops entries it already holds", () => {
    expect(seqs(applyPage(applyPage(emptyRunLog, page([1, 2, 3])), page([2, 3, 4])))).toEqual([1, 2, 3, 4]);
  });
  it("stops at a gap, so the next read asks again from the last held seq", () => {
    const v = applyPage(applyPage(emptyRunLog, page([1, 2])), page([3, 5, 6]));
    expect(seqs(v)).toEqual([1, 2, 3]);
    expect(v.headSeq).toBe(6);
  });
  it("keeps the core's break and write error", () => {
    const v = applyPage(emptyRunLog, { ...page([1]), broken: { seq: 2, why: "edited" }, io_error: "disk full" });
    expect([v.broken, v.ioError]).toEqual([{ seq: 2, why: "edited" }, "disk full"]);
  });
});

describe("followRunLog", () => {
  it("reads from 0, then only after the last seq on each notice", async () => {
    const { ipc, log } = core(3);
    let view = emptyRunLog;
    const stop = followRunLog("r", (v) => (view = v));
    await tick();
    expect(seqs(view)).toEqual([1, 2, 3]);
    log.n = 5;
    ipc.emit("run_log", { run_id: "r", seq: 5 });
    await tick();
    expect(seqs(view)).toEqual([1, 2, 3, 4, 5]);
    expect(ipc.calls.map((c) => c.args?.afterSeq)).toEqual([0, 3]);
    stop();
    expect(ipc.listenerCount("run_log")).toBe(0);
  });

  it("ignores other runs' notices and notices it already holds", async () => {
    const { ipc } = core(2);
    const stop = followRunLog("r", () => {});
    await tick();
    ipc.emit("run_log", { run_id: "other", seq: 9 });
    ipc.emit("run_log", { run_id: "r", seq: 2 });
    await tick();
    expect(ipc.calls.length).toBe(1);
    stop();
  });

  it("pages through a long log after a reload, in one read at a time", async () => {
    const { ipc } = core(12, 5);
    let view = emptyRunLog;
    const stop = followRunLog("r", (v) => (view = v));
    ipc.emit("run_log", { run_id: "r", seq: 12 });
    ipc.emit("run_log", { run_id: "r", seq: 12 });
    for (let i = 0; i < 6; i++) await tick();
    expect(seqs(view)).toEqual([...Array(12).keys()].map((i) => i + 1));
    expect(ipc.calls.map((c) => c.args?.afterSeq)).toEqual([0, 5, 10]);
    stop();
  });

  it("a reload from a kept view continues after its last seq", async () => {
    const { ipc } = core(4);
    const kept = applyPage(emptyRunLog, page([1, 2]));
    let view = kept;
    const stop = followRunLog("r", (v) => (view = v), kept);
    await tick();
    expect(seqs(view)).toEqual([1, 2, 3, 4]);
    expect(ipc.calls.map((c) => c.args?.afterSeq)).toEqual([2]);
    stop();
  });
});
