import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { setTransport } from "../src/ipc";
import type { StreamWindow, Tail } from "../src/ipc";
import { Crew } from "../src/screens/crew/Crew";
import { PANE_BYTES, appendTail } from "../src/screens/crew/streamAppend";
import { fakeIpc } from "./fakeIpc";
import { Live, liveRun } from "./liveRun";

afterEach(() => { cleanup(); setTransport(); window.location.hash = ""; });

const line = (seq: number, text = `out ${seq}`) => ({ seq, text });
const win = (from: number, to: number, extra: Partial<StreamWindow> = {}): StreamWindow => ({
  lines: Array.from({ length: to - from + 1 }, (_, i) => line(from + i)), head_seq: to, first_seq: 1, at_start: from <= 1, at_end: true, gap: false, ...extra,
});
const tail = (lines: { seq: number; text: string }[], head: number, extra: Partial<Tail> = {}): Tail => ({ lines, gap: false, head_seq: head, spill_start: null, spill_end: null, ...extra });

describe("appendTail", () => {
  it("appends the new lines to the held window and moves the head", () => {
    const next = appendTail(win(1, 3), tail([line(4), line(5)], 5), 100)!;
    expect(next.lines.map((l) => l.seq)).toEqual([1, 2, 3, 4, 5]);
    expect([next.head_seq, next.first_seq, next.at_start, next.at_end, next.gap]).toEqual([5, 1, true, true, false]);
  });

  it("answers the same window when nothing is new, so the pane does not redraw", () => {
    const held = win(1, 3);
    expect(appendTail(held, tail([], 3), 100)).toBe(held);
  });

  it("asks for a whole window when it cannot continue: a gap, a stream that restarted, or an answer that does not follow", () => {
    expect(appendTail(win(1, 3), tail([line(9)], 9, { gap: true }), 100)).toBeNull();
    expect(appendTail(win(1, 3), tail([], 1), 100)).toBeNull();
    expect(appendTail(win(1, 3), tail([line(2)], 3), 100)).toBeNull();
    expect(appendTail(win(1, 3), tail([], 7), 100)).toBeNull();
    expect(appendTail(win(1, 3, { at_end: false }), tail([line(4)], 4), 100)).toBeNull();
    expect(appendTail(win(1, 3, { gap: true }), tail([line(4)], 4), 100)).toBeNull();
  });

  it("caps the pane by rows: the oldest rows drop and the pane is no longer at the start", () => {
    const next = appendTail(win(1, 5), tail([line(6), line(7), line(8)], 8), 5)!;
    expect(next.lines.map((l) => l.seq)).toEqual([4, 5, 6, 7, 8]);
    expect([next.first_seq, next.at_start, next.at_end]).toEqual([1, false, true]);
  });

  it("caps the pane by bytes and always keeps the newest row, even when one row is over the cap", () => {
    const big = "x".repeat(600);
    const next = appendTail(win(1, 2), tail([line(3, big), line(4, big)], 4), 100, 1000)!;
    expect(next.lines.map((l) => l.seq)).toEqual([4]);
    const one = appendTail(win(1, 1), tail([line(2, "y".repeat(5000))], 2), 100, 1000)!;
    expect(one.lines.map((l) => l.seq)).toEqual([2]);
  });

  it("the default byte cap is 1 MiB, the size of a whole window", () => {
    expect(PANE_BYTES).toBe(1024 * 1024);
  });

  it("a 30 KB reply streamed in 100 chunks sends at most 40 KB, where re-reading the window each time sends megabytes", () => {
    const chunk = "z".repeat(300);
    let held = win(1, 1);
    let sent = JSON.stringify(held).length;
    let rereads = 0;
    const all = [line(1)];
    for (let i = 2; i <= 101; i++) {
      all.push(line(i, chunk));
      const t = tail([line(i, chunk)], i);
      sent += JSON.stringify(t).length;
      held = appendTail(held, t, 1000)!;
      rereads += JSON.stringify(win(1, i, { lines: all.slice() })).length;
    }
    expect(held.lines).toHaveLength(101);
    expect(sent).toBeLessThanOrEqual(40 * 1024);
    expect(rereads).toBeGreaterThan(10 * sent);
  });
});

describe("Stream tab following the live end", () => {
  const node = (id: string, kind: string, title: string, children: unknown[] = []) => ({ id, kind, title, engine_id: null, status: "in_progress", overlays: [], children });
  const tree = node("lead", "lead", "Lead", [node("run", "run", "Run r1", [node("agent:a1", "worker", "Backend")])]);

  it("asks only for the lines after the last one it holds, and shows them", async () => {
    window.location.hash = "#/crew?slug=camp&id=r1";
    let head = 3;
    const f = fakeIpc({
      ...liveRun(),
      crew: tree,
      stream_window: () => ({ ...win(1, head), head_seq: head }),
      tail: (a?: Record<string, unknown>) => tail(Array.from({ length: head - (a?.afterSeq as number) }, (_, i) => line((a?.afterSeq as number) + 1 + i)), head),
    });
    setTransport(f.transport);
    render(<Live><Crew /></Live>);
    fireEvent.click((await screen.findByText("Backend")).closest("[role=treeitem]") as HTMLElement);
    const log = await screen.findByRole("log");
    await waitFor(() => expect(log.textContent).toContain("out 3"));
    const windows = f.calls.filter((c) => c.cmd === "stream_window").length;
    head = 5;
    act(() => f.emit("changed", { streams: ["w-a1"], snapshot_needed: false }));
    await waitFor(() => expect(log.textContent).toContain("out 5"));
    const tails = f.calls.filter((c) => c.cmd === "tail");
    expect(tails.at(-1)?.args).toMatchObject({ stream: "w-a1", afterSeq: 3 });
    expect(f.calls.filter((c) => c.cmd === "stream_window")).toHaveLength(windows);
  });
});
