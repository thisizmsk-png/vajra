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

  // A 10 MiB spill (well over the old 4 MiB page budget) whose lines 1..30000 are before the window, which starts at 30001.
  const spillBefore = (args?: Record<string, unknown>) => {
    const before = args?.beforeSeq as number, max = args?.maxLines as number;
    const from = Math.max(1, before - max);
    return { lines: lines(from, before - 1), at_start: from === 1, gap: false };
  };
  const earlierButton = () => screen.queryByRole("button", { name: S.crew.loadEarlier });

  it("loads the lines immediately before the window, then walks back contiguously", async () => {
    const f = await openBackend({
      tail: tail(lines(30001, 30003), { spill_start: 0, spill_end: 10 * 1024 * 1024 }),
      read_spill_before: spillBefore,
    });
    const asks = () => f.calls.filter((c) => c.cmd === "read_spill_before").map((c) => c.args);
    fireEvent.click(await screen.findByRole("button", { name: S.crew.loadEarlier }));
    await waitFor(() => expect(asks()).toHaveLength(1));
    expect(asks()[0]).toEqual({ stream: "w-a1", beforeSeq: 30001, maxLines: 1000 });
    for (let k = 2; k <= 5; k++) {
      await waitFor(() => expect(earlierButton()).toBeTruthy());
      fireEvent.click(earlierButton()!);
      await waitFor(() => expect(asks()).toHaveLength(k));
      // Each click starts where the previous page started: contiguous, never from the start of the spill.
      expect(asks()[k - 1]).toEqual({ stream: "w-a1", beforeSeq: 30001 - 1000 * (k - 1), maxLines: 1000 });
    }
    expect(f.calls.some((c) => c.cmd === "read_spill")).toBe(false);
  });

  it("is done only when the start of the spill is reached", async () => {
    const f = await openBackend({
      tail: tail(lines(3001, 3003), { spill_start: 0, spill_end: 1024 }),
      read_spill_before: spillBefore,
    });
    const asks = () => f.calls.filter((c) => c.cmd === "read_spill_before").length;
    for (let k = 1; k <= 3; k++) {
      fireEvent.click(await screen.findByRole("button", { name: S.crew.loadEarlier }));
      await waitFor(() => expect(asks()).toBe(k));
      if (k < 3) await waitFor(() => expect(earlierButton()).toBeTruthy());
    }
    await waitFor(() => expect(earlierButton()).toBeNull());
  });

  it("keeps the button while older output exists, even after the first page", async () => {
    await openBackend({
      tail: tail(lines(30001, 30003), { spill_start: 0, spill_end: 10 * 1024 * 1024 }),
      read_spill_before: spillBefore,
    });
    fireEvent.click(await screen.findByRole("button", { name: S.crew.loadEarlier }));
    await waitFor(() => expect(screen.getByRole("log").textContent).toBeTruthy());
    await waitFor(() => expect(earlierButton()).toBeTruthy());
  });

  it("says where the view stops when the row cap is reached", async () => {
    await openBackend({
      tail: tail(lines(900001, 900003), { spill_start: 0, spill_end: 10 }),
      read_spill_before: spillBefore,
    });
    for (let k = 0; k < 19; k++) {
      await waitFor(() => expect(earlierButton()).toBeTruthy());
      fireEvent.click(earlierButton()!);
      await waitFor(() => expect(earlierButton()?.hasAttribute("disabled") ?? false).toBe(false));
    }
    expect(await screen.findByText(S.crew.earlierLimit)).toBeTruthy();
    expect(earlierButton()).toBeNull();
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
