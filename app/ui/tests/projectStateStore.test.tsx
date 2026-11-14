// FG-06 / ADR-15 §4: the board reads one revisioned project state. Given a burst of `changed` events, when the
// revision moves, then it is fetched once per 500 ms gap and nothing re-renders for an unchanged answer.
import { act, cleanup, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { setTransport } from "../src/ipc";
import { useProjectState } from "../src/projects/state";
import { fakeIpc } from "./fakeIpc";

const full = (rev: number, id = "p_1") => ({ schema: "vajra.project_state/1", project_id: id, rev, unchanged: false, active_runs: 0, generations: { project: 0 }, workers: [], receipts: [] });
const same = (rev: number, id = "p_1") => ({ schema: "vajra.project_state/1", project_id: id, rev, unchanged: true });
const changed = (rev: number, id = "p_1") => ({ streams: ["workers"], snapshot_needed: false, projects: [{ project_id: id, rev }] });

let renders: Array<{ rev: number | null; failed: boolean; usage?: unknown }> = [];
function Probe({ id }: { id: string | null }) {
  const s = useProjectState(id);
  renders.push(s);
  return null;
}
const stateCalls = (calls: { cmd: string }[]) => calls.filter((c) => c.cmd === "project_state");
const flush = () => act(async () => { await vi.advanceTimersByTimeAsync(0); });
const tick = (ms: number) => act(async () => { await vi.advanceTimersByTimeAsync(ms); });

beforeEach(() => { vi.useFakeTimers(); renders = []; });
afterEach(() => { cleanup(); setTransport(); vi.useRealTimers(); });

describe("useProjectState", () => {
  it("reads the revision once on mount", async () => {
    const f = fakeIpc({ project_state: () => full(10) });
    setTransport(f.transport);
    render(<Probe id="p_1" />);
    await flush();
    expect(renders.at(-1)).toEqual({ rev: 10, failed: false, proposals: null, usage: null });
    expect(stateCalls(f.calls)).toHaveLength(1);
  });

  it("shares one subscription and one fetch between components on the same project", async () => {
    const f = fakeIpc({ project_state: () => full(10) });
    setTransport(f.transport);
    render(<><Probe id="p_1" /><Probe id="p_1" /></>);
    await flush();
    expect(f.listenerCount("changed")).toBe(1);
    expect(stateCalls(f.calls)).toHaveLength(1);
  });

  it("fetches a burst of events once per gap and shows the newest revision", async () => {
    let rev = 10;
    const f = fakeIpc({ project_state: () => full(rev) });
    setTransport(f.transport);
    render(<Probe id="p_1" />);
    await flush();
    await tick(600);
    rev = 14;
    for (let r = 11; r <= 14; r++) f.emit("changed", changed(r));
    await flush();
    expect(stateCalls(f.calls)).toHaveLength(2); // one on mount, one for the whole burst
    expect(renders.at(-1)?.rev).toBe(14);
  });

  it("holds a second fetch back until the gap has passed", async () => {
    let rev = 10;
    const f = fakeIpc({ project_state: () => full(rev) });
    setTransport(f.transport);
    render(<Probe id="p_1" />);
    await flush();
    await tick(600);
    rev = 11; f.emit("changed", changed(11)); await flush();
    rev = 12; f.emit("changed", changed(12)); await flush();
    expect(stateCalls(f.calls)).toHaveLength(2);
    await tick(500);
    expect(stateCalls(f.calls)).toHaveLength(3);
    expect(renders.at(-1)?.rev).toBe(12);
  });

  it("ignores events about other projects and revisions it already holds", async () => {
    const f = fakeIpc({ project_state: () => full(10) });
    setTransport(f.transport);
    render(<Probe id="p_1" />);
    await flush();
    await tick(600);
    f.emit("changed", changed(99, "p_other"));
    f.emit("changed", changed(10));
    await tick(600);
    expect(stateCalls(f.calls)).toHaveLength(1);
  });

  it("does not re-render when the answer is unchanged", async () => {
    const f = fakeIpc({ project_state: (a?: Record<string, unknown>) => (a?.sinceRev === 10 ? same(10) : full(10)) });
    setTransport(f.transport);
    render(<Probe id="p_1" />);
    await flush();
    await tick(600);
    const before = renders.length;
    f.emit("changed", { streams: ["workers"], snapshot_needed: false }); // no projects list: one look is enough
    await flush();
    expect(stateCalls(f.calls)).toHaveLength(2);
    expect(renders.length).toBe(before);
  });

  it("flags a failure instead of hiding it, and recovers on the next good answer", async () => {
    let bad = true;
    const f = fakeIpc({ project_state: () => { if (bad) throw { error: "unknown_command" }; return full(20); } });
    setTransport(f.transport);
    render(<Probe id="p_1" />);
    await flush();
    expect(renders.at(-1)).toEqual({ rev: null, failed: true, proposals: null, usage: null });
    await tick(600);
    bad = false;
    f.emit("changed", { streams: ["workers"], snapshot_needed: false });
    await flush();
    expect(renders.at(-1)).toEqual({ rev: 20, failed: false, proposals: null, usage: null });
  });

  it("keeps the proposals it has through a failed refresh, so recovery at the same revision still shows them (#90 review)", async () => {
    const item = { id: "proposal:a", kind: "run_proposal", origin: "app", created: "", age_s: 1, title: "Start a run?", body: "go", actions: ["start", "dismiss"] };
    let mode: "full" | "bad" | "same" = "full";
    const f = fakeIpc({ project_state: () => {
      if (mode === "bad") throw { error: "io_error" };
      return mode === "same" ? same(30) : { ...full(30), proposals: { ok: true, count: 1, more: 0, items: [item] } };
    } });
    setTransport(f.transport);
    render(<Probe id="p_1" />);
    await flush();
    expect(renders.at(-1)).toEqual({ rev: 30, failed: false, proposals: [item], usage: null });
    await tick(600);
    mode = "bad";
    f.emit("changed", { streams: ["workers"], snapshot_needed: false });
    await flush();
    expect(renders.at(-1)).toEqual({ rev: 30, failed: true, proposals: [item], usage: null });
    await tick(600);
    mode = "same";
    f.emit("changed", { streams: ["workers"], snapshot_needed: false });
    await flush();
    expect(renders.at(-1)).toEqual({ rev: 30, failed: false, proposals: [item], usage: null });
  });

  it("lets go of the subscription when the last user unmounts", async () => {
    const f = fakeIpc({ project_state: () => full(10) });
    setTransport(f.transport);
    const { unmount } = render(<Probe id="p_1" />);
    await flush();
    unmount();
    expect(f.listenerCount("changed")).toBe(0);
  });

  it("does nothing without a project", async () => {
    const f = fakeIpc({});
    setTransport(f.transport);
    render(<Probe id={null} />);
    await flush();
    expect(f.calls).toHaveLength(0);
    expect(renders.at(-1)).toEqual({ rev: null, failed: false, proposals: null, usage: null });
  });

  it("carries the usage answers across an unchanged revision and drops one that is not the contract's shape", async () => {
    const usage = { schema: "vajra.provider_usage/1", usage: [{ account: "claude", read_at: null, source: "none", windows: [{ kind: "weekly", used_pct: 84, resets_at: null, warn: true }], amounts: null }] };
    let answer: unknown = { ...full(10), usage };
    const f = fakeIpc({ project_state: () => answer });
    setTransport(f.transport);
    render(<Probe id="p_1" />);
    await flush();
    expect((renders.at(-1)?.usage as { usage: { account: string }[] }).usage[0].account).toBe("claude");
    await tick(600);
    answer = same(10);
    f.emit("changed", { streams: [], snapshot_needed: true });
    await flush();
    expect(renders.at(-1)?.usage).not.toBeNull();
    await tick(600);
    answer = { ...full(11), usage: { schema: "vajra.provider_usage/9", usage: [] } };
    f.emit("changed", changed(11));
    await flush();
    expect(renders.at(-1)?.usage).toBeNull();
  });
});
