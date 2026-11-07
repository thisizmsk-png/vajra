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

/** A core holding lines first..head: `stream_window` as the real command answers it. */
function core(head: number, first = 1, extra: object = {}) {
  return (a?: Record<string, unknown>) => {
    const from = a?.from as number | null, max = a?.maxLines as number;
    const start = from === null ? Math.max(first, head - max + 1) : Math.max(first, from);
    const end = Math.min(head, start + max - 1);
    return { lines: lines(start, end), head_seq: head, first_seq: first, at_start: start <= first, at_end: end >= head, gap: from !== null && from < first, ...extra };
  };
}

async function openBackend(replies: Record<string, unknown>) {
  window.location.hash = "#/crew?slug=camp&id=r1";
  const f = fakeIpc({ active_runs: { count: 0, runs: [] }, crew: tree, ...replies });
  setTransport(f.transport);
  render(<Crew />);
  fireEvent.click((await screen.findByText("Backend")).closest("[role=treeitem]") as HTMLElement);
  return f;
}
const wins = (f: ReturnType<typeof fakeIpc>) => f.calls.filter((c) => c.cmd === "stream_window");
const scrollTo = (rows: number) => {
  const log = screen.getByRole("log");
  act(() => { log.scrollTop = rows * 18; fireEvent.scroll(log); });
};

describe("Stream tab: a window served by the core (#45)", () => {
  it("shows the selected worker's output, read only, from the worker's stream", async () => {
    const f = await openBackend({ stream_window: core(3) });
    const log = await screen.findByRole("log", { name: S.crew.streamLabel });
    await waitFor(() => expect(log.textContent).toContain("out 3"));
    expect(wins(f)[0].args).toMatchObject({ stream: "w-a1", from: null });
  });

  it("shows no stream for the Lead or the run (they have no worker stream)", async () => {
    window.location.hash = "#/crew?slug=camp&id=r1";
    const f = fakeIpc({ active_runs: { count: 0, runs: [] }, crew: tree, stream_window: core(3) });
    setTransport(f.transport);
    render(<Crew />);
    await screen.findByRole("tree");
    expect(screen.queryByRole("log")).toBeNull();
    expect(wins(f)).toHaveLength(0);
  });

  it("follows the tail by replacing the window on each poll: no cursor, no draining loop", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    let head = 3;
    const f = await openBackend({ stream_window: (a?: Record<string, unknown>) => core(head)(a) });
    await screen.findByRole("log");
    head = 9;
    await act(async () => { await vi.advanceTimersByTimeAsync(1100); });
    await waitFor(() => expect(screen.getByRole("log").textContent).toContain("out 9"));
    expect(wins(f).every((c) => c.args?.from === null)).toBe(true);
    expect(wins(f).length).toBeLessThan(5);
  });

  it("holds a bounded window and renders only the rows in view for a very long stream", async () => {
    const f = await openBackend({ stream_window: core(100000) });
    const log = await screen.findByRole("log");
    await waitFor(() => expect(log.textContent).toContain("out 100000"));
    expect(document.querySelectorAll(".v-vrow").length).toBeLessThan(80);
    expect(wins(f)[0].args?.maxLines as number).toBeLessThanOrEqual(500);
  });

  it("asks for the rows around the view when scrolling reaches the window edge", async () => {
    const f = await openBackend({ stream_window: core(5000) });
    const log = await screen.findByRole("log");
    await waitFor(() => expect(log.textContent).toContain("out 5000"));
    scrollTo(2000); // far outside the held tail window: placeholders until the answer lands
    await waitFor(() => expect(wins(f).some((c) => c.args?.from !== null)).toBe(true));
    const req = wins(f).find((c) => c.args?.from !== null)!;
    expect(req.args?.from as number).toBeGreaterThan(1850);
    expect(req.args?.from as number).toBeLessThanOrEqual(2001);
    await waitFor(() => expect(screen.getByRole("log").textContent).toContain("out 2001"));
    expect(screen.getByText(S.crew.historyView)).toBeTruthy();
  });

  it("does not fetch while the view sits inside the held window", async () => {
    const f = await openBackend({ stream_window: core(5000) });
    const log = await screen.findByRole("log");
    await waitFor(() => expect(log.textContent).toContain("out 5000"));
    const n = wins(f).length;
    scrollTo(4960 - 5); // a little above the bottom, still inside the 220 held rows
    await new Promise((r) => setTimeout(r, 100));
    expect(wins(f).filter((c) => c.args?.from !== null)).toHaveLength(0);
    expect(wins(f).length).toBeLessThanOrEqual(n + 1);
  });

  it("drops a scroll answer that arrives after Jump to latest (a stale window never replaces the new one)", async () => {
    let release: (v: unknown) => void = () => {};
    const slow = new Promise((r) => { release = r; });
    const f = await openBackend({
      stream_window: (a?: Record<string, unknown>) => (a?.from === null ? core(5000)(a) : slow.then(() => core(5000)(a))),
    });
    const log = await screen.findByRole("log");
    await waitFor(() => expect(log.textContent).toContain("out 5000"));
    scrollTo(1000);
    await waitFor(() => expect(wins(f).some((c) => c.args?.from !== null)).toBe(true));
    fireEvent.click(screen.getByRole("button", { name: S.crew.jumpLatest }));
    await waitFor(() => expect(screen.getByRole("log").textContent).toContain("out 5000"));
    await act(async () => { release(null); await Promise.resolve(); });
    expect(screen.getByRole("log").textContent).toContain("out 5000");
    expect(screen.getByRole("log").textContent).not.toContain("out 1001");
  });

  it("marks a window that lost lines", async () => {
    await openBackend({ stream_window: core(30, 10, { gap: true }) });
    await screen.findByRole("log");
    await waitFor(() => expect(screen.getByText(S.crew.windowGap)).toBeTruthy());
  });

  it("stops asking for older rows once the core says the window starts the history", async () => {
    const f = await openBackend({ stream_window: core(300) });
    const log = await screen.findByRole("log");
    await waitFor(() => expect(log.textContent).toContain("out 300"));
    scrollTo(0);
    await waitFor(() => expect(log.textContent).toContain("out 1"));
    const n = wins(f).filter((c) => c.args?.from !== null).length;
    scrollTo(0);
    await new Promise((r) => setTimeout(r, 150));
    expect(wins(f).filter((c) => c.args?.from !== null).length).toBe(n);
  });

  it("reaches line 1 of a stream much longer than the window by scrolling up", async () => {
    const f = await openBackend({ stream_window: core(5000) });
    const log = await screen.findByRole("log");
    await waitFor(() => expect(log.textContent).toContain("out 5000"));
    scrollTo(0);
    await waitFor(() => expect(screen.getByRole("log").textContent).toContain("out 1"));
    expect(wins(f).some((c) => c.args?.from === 1)).toBe(true);
    expect(screen.getByRole("log").textContent).toContain("out 1");
  });

  it("keeps the same lines in view when first_seq advances under a reader who scrolled up", async () => {
    let first = 1;
    const f = await openBackend({ stream_window: (a?: Record<string, unknown>) => core(5000, first)(a) });
    const log = await screen.findByRole("log");
    await waitFor(() => expect(log.textContent).toContain("out 5000"));
    scrollTo(2000); // top row is seq 2001
    await waitFor(() => expect(screen.getByRole("log").textContent).toContain("out 2001"));
    const n = wins(f).length;
    first = 501; // the core evicted 500 lines
    await new Promise((r) => setTimeout(r, 600));
    act(() => f.emit("changed", { streams: ["w-a1"], snapshot_needed: false }));
    await waitFor(() => expect(wins(f).length).toBeGreaterThan(n));
    // seq 2001 is now row 1500, not row 2000
    await waitFor(() => expect((screen.getByRole("log") as HTMLElement).scrollTop).toBe(1500 * 18));
    expect(screen.getByRole("log").textContent).toContain("out 2001");
  });

  it("keeps one request in flight", async () => {
    let inflight = 0, peak = 0;
    await openBackend({
      stream_window: async (a?: Record<string, unknown>) => {
        inflight++; peak = Math.max(peak, inflight);
        await new Promise((r) => setTimeout(r, 30));
        inflight--;
        return core(5000)(a);
      },
    });
    const log = await screen.findByRole("log");
    await waitFor(() => expect(log.textContent).toContain("out 5000"));
    scrollTo(1000); scrollTo(2000); scrollTo(3000);
    await new Promise((r) => setTimeout(r, 200));
    expect(peak).toBe(1);
  });

  it("says so when the output cannot be read", async () => {
    await openBackend({ stream_window: new Error("boom") });
    expect((await screen.findByText(new RegExp(S.crew.streamFailed))).closest("[role=alert]")).toBeTruthy();
  });

  it("re-reads on a changed event for its own stream, and ignores one for another stream", async () => {
    const f = await openBackend({ stream_window: core(3) });
    await screen.findByRole("log");
    await waitFor(() => expect(wins(f).length).toBeGreaterThan(0));
    const before = wins(f).length;
    act(() => f.emit("changed", { streams: ["w-other"], snapshot_needed: false }));
    await new Promise((r) => setTimeout(r, 700));
    expect(wins(f).length).toBe(before);
    act(() => f.emit("changed", { streams: ["w-a1"], snapshot_needed: false }));
    await waitFor(() => expect(wins(f).length).toBeGreaterThan(before));
  });
});
