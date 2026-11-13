// W2 live updates: the run view applies each notice's entries by seq with no call, and calls `run_log_since` only to
// start, after a reload, or to fill a gap. A WebView reload loses nothing.
import { afterEach, describe, expect, it } from "vitest";
import { setTransport, type RunLogEntry, type RunLogNotice } from "../src/ipc";
import { applyNotice, applyPage, emptyRunLog, followRunLog, type RunLogStats, type RunLogView } from "../src/runLog";
import { fakeIpc } from "./fakeIpc";

afterEach(() => setTransport());

const NOTICE_MAX = 64;
const entry = (seq: number, worker = 0): RunLogEntry => ({ v: 1, seq, prev: `h${seq - 1}`, at_ms: seq, source: { worker: `k${worker}` }, kind: "ticket_ended", body: {}, hash: `h${seq}` });
const page = (seqs: number[], head = Math.max(0, ...seqs)) => ({ run_id: "r", entries: seqs.map((s) => entry(s)), head_seq: head, broken: null, io_error: null });
const seqs = (v: RunLogView) => v.entries.map((e) => e.seq);
const tick = () => new Promise((r) => setTimeout(r, 0));
const settle = async () => { for (let i = 0; i < 8; i++) await tick(); };

/** A fake core: a log written up to `n`, `run_log_since` like the core's (after N, at most `cap`), and notices like the
 *  core's: each carries the written entries after the last notice, at most 64. */
function core(cap = 500) {
  const entries: RunLogEntry[] = [];
  let told = 0;
  const ipc = fakeIpc({
    run_log_since: (a?: Record<string, unknown>) => {
      const after = Number(a?.afterSeq ?? 0);
      return { run_id: "r", entries: entries.slice(after, after + cap), head_seq: entries.length, broken: null, io_error: null };
    },
  });
  setTransport(ipc.transport);
  const write = (count: number, workers = 4) => {
    for (let i = 0; i < count; i++) entries.push(entry(entries.length + 1, entries.length % workers));
  };
  /** Tell what is written since the last notice, as the core does. `drop`: the notice is lost on the way. */
  const tell = (drop = false) => {
    const n: RunLogNotice = { run_id: "r", seq: entries.length, after: told, entries: entries.slice(told, told + NOTICE_MAX) };
    told = entries.length;
    if (!drop) ipc.emit("run_log", n);
  };
  const calls = () => ipc.calls.filter((c) => c.cmd === "run_log_since").length;
  return { ipc, write, tell, calls, entries };
}

describe("applyNotice", () => {
  it("applies a notice that follows the view, with no gap", () => {
    const v = applyPage(emptyRunLog, page([1, 2]));
    const { view, gap } = applyNotice(v, { run_id: "r", seq: 4, after: 2, entries: [entry(3), entry(4)] });
    expect([seqs(view), gap]).toEqual([[1, 2, 3, 4], false]);
  });
  it("reports a gap when the notice starts past the view", () => {
    const v = applyPage(emptyRunLog, page([1, 2]));
    const { view, gap } = applyNotice(v, { run_id: "r", seq: 6, after: 4, entries: [entry(5), entry(6)] });
    expect([seqs(view), gap]).toEqual([[1, 2], true]);
  });
  it("reports a gap when a notice was cut at the cap", () => {
    const { view, gap } = applyNotice(emptyRunLog, { run_id: "r", seq: 70, after: 0, entries: [1, 2, 3].map((s) => entry(s)) });
    expect([seqs(view), gap]).toEqual([[1, 2, 3], true]);
  });
  it("drops entries it already holds", () => {
    const v = applyPage(emptyRunLog, page([1, 2, 3]));
    expect(seqs(applyNotice(v, { run_id: "r", seq: 4, after: 1, entries: [2, 3, 4].map((s) => entry(s)) }).view)).toEqual([1, 2, 3, 4]);
  });
});

describe("followRunLog in a 4-worker run", () => {
  it("makes one call to start and none while notices follow on", async () => {
    const c = core();
    c.write(3);
    const stats: RunLogStats = { notices: 0, calls: 0 };
    let view = emptyRunLog;
    const stop = followRunLog("r", (v) => (view = v), emptyRunLog, stats);
    await settle();
    c.tell(true); // the core's notice for the first 3 raced the start read, which already has them
    expect(c.calls()).toBe(1);
    for (let round = 0; round < 50; round++) {
      c.write(1 + (round % 5)); // 1–5 entries per notice, from 4 workers
      c.tell();
      await tick();
    }
    await settle();
    expect(seqs(view)).toEqual(c.entries.map((e) => e.seq));
    expect(c.calls()).toBe(1);
    expect(stats.notices).toBe(50);
    stop();
  });

  it("makes exactly one call for each gap, never one per notice", async () => {
    const c = core();
    let view = emptyRunLog;
    const stop = followRunLog("r", (v) => (view = v));
    await settle();
    c.write(4); c.tell();
    c.write(4); c.tell(true); // lost
    c.write(4); c.tell(); // starts after 8: a gap
    await settle();
    expect(c.calls()).toBe(2);
    c.write(NOTICE_MAX + 20); c.tell(); // one burst, cut at the cap
    await settle();
    expect(c.calls()).toBe(3);
    for (let i = 0; i < 10; i++) { c.write(2); c.tell(); }
    await settle();
    expect(c.calls()).toBe(3);
    expect(seqs(view)).toEqual(c.entries.map((e) => e.seq));
    stop();
  });

  it("a lost or oversized notice always heals by refetching, and no entry is ever dropped", async () => {
    const c = core(5); // pages of 5, so healing an oversized gap takes several reads
    let view = emptyRunLog;
    const stop = followRunLog("r", (v) => (view = v));
    await settle();
    const all = () => c.entries.map((e) => e.seq);
    c.write(3); c.tell(true); // lost
    c.write(2); c.tell(); // past the view: heals
    await settle();
    expect(seqs(view)).toEqual(all());
    c.write(NOTICE_MAX * 2); c.tell(); // oversized: carries 64 of 128, the rest is read in pages
    await settle();
    expect(seqs(view)).toEqual(all());
    // A refetch that fails keeps what the view has; the next notice past it heals it.
    const real = c.ipc.transport.call;
    let fail = true;
    c.ipc.transport.call = async (cmd, args) => {
      if (cmd === "run_log_since" && fail) { fail = false; throw { error: "io", detail: "busy" }; }
      return real(cmd, args);
    };
    c.write(4); c.tell(true); // lost
    c.write(1); c.tell(); // gap; this refetch fails
    await settle();
    expect(seqs(view).length).toBeLessThan(all().length);
    c.write(1); c.tell(); // still a gap: refetched, healed
    await settle();
    expect(seqs(view)).toEqual(all());
    // Never a hole: the view is always 1, 2, 3, ... in order.
    expect(seqs(view)).toEqual(seqs(view).map((_, i) => i + 1));
    stop();
  });

  it("ignores other runs and stops listening when stopped", async () => {
    const c = core();
    const stop = followRunLog("r", () => {});
    await settle();
    c.ipc.emit("run_log", { run_id: "other", seq: 9, after: 0, entries: [] });
    await settle();
    expect(c.calls()).toBe(1);
    stop();
    expect(c.ipc.listenerCount("run_log")).toBe(0);
  });

  it("a reload loses nothing: from a kept view or from 0, the list is the same", async () => {
    const c = core(5);
    c.write(12);
    let first = emptyRunLog;
    const stop1 = followRunLog("r", (v) => (first = v));
    await settle();
    stop1(); // the WebView goes away
    c.write(7); // the run goes on meanwhile; those notices reach no one
    c.tell(true);
    let fresh = emptyRunLog;
    const stop2 = followRunLog("r", (v) => (fresh = v));
    let kept = first;
    const stop3 = followRunLog("r", (v) => (kept = v), first);
    await settle();
    expect(seqs(fresh)).toEqual(c.entries.map((e) => e.seq));
    expect(seqs(kept)).toEqual(c.entries.map((e) => e.seq));
    stop2(); stop3();
  });
});
