import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { setTransport } from "../src/ipc";
import { Crew } from "../src/screens/crew/Crew";
import { S } from "../strings";
import { fakeIpc } from "./fakeIpc";

afterEach(() => {
  cleanup();
  setTransport();
  window.location.hash = "";
  vi.useRealTimers();
});

const node = (id: string, kind: string, title: string, status: string, children: unknown[] = []) =>
  ({ id, kind, title, engine_id: null, status, overlays: [], children });
const tree = node("lead", "lead", "Lead", "in_progress", [
  node("run", "run", "Run r1", "in_progress", [node("agent:a1", "worker", "Backend", "in_progress")]),
]);
const lines = (from: number, to: number) => Array.from({ length: to - from + 1 }, (_, i) => ({ seq: from + i, text: `out ${from + i}` }));
const tail = (ls: { seq: number; text: string }[], extra: object = {}) =>
  ({ lines: ls, gap: false, head_seq: ls.at(-1)?.seq ?? 0, spill_start: null, spill_end: null, ...extra });

async function openBackend(replies: Record<string, unknown>) {
  window.location.hash = "#/crew?slug=camp&id=r1";
  const f = fakeIpc({ active_runs: { count: 0, runs: [] }, crew: tree, ...replies });
  setTransport(f.transport);
  render(<Crew />);
  fireEvent.click((await screen.findByText("Backend")).closest("[role=treeitem]") as HTMLElement);
  return f;
}
const tails = (f: ReturnType<typeof fakeIpc>) => f.calls.filter((c) => c.cmd === "tail");

describe("Stream tab (S2b, US-2.2)", () => {
  it("shows the selected worker's output, read only, from the worker's stream", async () => {
    const f = await openBackend({ tail: tail(lines(1, 3)) });
    const log = await screen.findByRole("log", { name: S.crew.streamLabel });
    await waitFor(() => expect(log.textContent).toContain("out 1"));
    expect(tails(f)[0].args).toMatchObject({ stream: "w-a1", afterSeq: 0 });
  });

  it("shows no stream for the Lead or the run (they have no worker stream)", async () => {
    window.location.hash = "#/crew?slug=camp&id=r1";
    const f = fakeIpc({ active_runs: { count: 0, runs: [] }, crew: tree, tail: tail([]) });
    setTransport(f.transport);
    render(<Crew />);
    await screen.findByRole("tree");
    expect(screen.queryByRole("log")).toBeNull();
    expect(tails(f)).toHaveLength(0);
  });

  it("asks only for what is new: the next tail starts after the last seq seen", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const f = await openBackend({ tail: (a?: Record<string, unknown>) => ((a?.afterSeq as number) === 0 ? tail(lines(1, 3)) : tail([])) });
    await screen.findByRole("log");
    await act(async () => { await vi.advanceTimersByTimeAsync(1100); });
    const t = tails(f);
    expect(t.length).toBeGreaterThan(1);
    expect(t[1].args).toMatchObject({ afterSeq: 3 });
  });

  it("keeps the DOM small for a long stream: only the rows in view are rendered", async () => {
    await openBackend({ tail: tail(lines(1, 3000)) });
    await screen.findByRole("log");
    await waitFor(() => expect(document.querySelectorAll(".v-vrow").length).toBeGreaterThan(0));
    expect(document.querySelectorAll(".v-vrow").length).toBeLessThan(80);
  });

  it("marks output that fell off the core's ring", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    let n = 0;
    await openBackend({ tail: () => (n++ === 0 ? tail(lines(1, 3)) : tail(lines(10, 12), { gap: true })) });
    await screen.findByRole("log");
    await act(async () => { await vi.advanceTimersByTimeAsync(1100); });
    await waitFor(() => expect(screen.getByRole("log").textContent).toContain("6 lines were skipped"));
  });

  it("loads earlier output from the spill on request, once", async () => {
    const f = await openBackend({
      tail: tail(lines(100, 102), { spill_start: 0, spill_end: 50 }),
      read_spill: { lines: lines(1, 5), next_offset: 50, gap: false, end: true },
    });
    const btn = await screen.findByRole("button", { name: S.crew.loadEarlier });
    fireEvent.click(btn);
    await waitFor(() => expect(screen.getByRole("log").textContent).toContain("out 1"));
    expect(f.calls.find((c) => c.cmd === "read_spill")?.args).toMatchObject({ stream: "w-a1", offset: 0 });
    expect(screen.queryByRole("button", { name: S.crew.loadEarlier })).toBeNull();
  });

  it("offers no 'load earlier' when nothing was spilled", async () => {
    await openBackend({ tail: tail(lines(1, 3), { spill_start: 0, spill_end: 0 }) });
    await screen.findByRole("log");
    expect(screen.queryByRole("button", { name: S.crew.loadEarlier })).toBeNull();
  });

  it("says so when the output cannot be read", async () => {
    await openBackend({ tail: new Error("boom") });
    expect((await screen.findByText(new RegExp(S.crew.streamFailed))).closest("[role=alert]")).toBeTruthy();
  });

  it("re-reads on a changed event for its own stream, and ignores one for another stream", async () => {
    const f = await openBackend({ tail: tail(lines(1, 3)) });
    await screen.findByRole("log");
    await waitFor(() => expect(tails(f).length).toBeGreaterThan(0));
    const before = tails(f).length;
    act(() => f.emit("changed", { streams: ["w-other"], snapshot_needed: false }));
    await new Promise((r) => setTimeout(r, 700));
    expect(tails(f).length).toBe(before);
    act(() => f.emit("changed", { streams: ["w-a1"], snapshot_needed: false }));
    await waitFor(() => expect(tails(f).length).toBeGreaterThan(before));
  });
});
