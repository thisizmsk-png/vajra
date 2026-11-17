import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { setTransport } from "../src/ipc";
import type { StreamWindow } from "../src/ipc";
import { LOAD_END_S, LOAD_START_S, TOTAL, delivered, fixtureTransport } from "../src/perf/fixture";
import { Crew } from "../src/screens/crew/Crew";
import { S } from "../strings";

afterEach(() => {
  cleanup();
  setTransport();
  window.location.hash = "";
});

const win = async (elapsed: number, args: Record<string, unknown>) => {
  const { transport } = fixtureTransport(() => elapsed);
  return (await transport.call("stream_window", { stream: "w-a1", ...args })) as StreamWindow;
};

describe("perf fixture", () => {
  it("delivers nothing before the load, all TOTAL lines by LOAD_END_S, and never more", () => {
    expect(delivered(0)).toBe(0);
    expect(delivered(LOAD_START_S)).toBe(0);
    expect(delivered(LOAD_END_S)).toBe(TOTAL);
    expect(delivered(LOAD_END_S + 1000)).toBe(TOTAL);
  });

  it("answers stream_window with the real StreamWindow shape: the newest rows, then rows from a seq, clamped to 500", async () => {
    const newest = await win(LOAD_END_S + 1, { from: null, maxLines: 220 });
    expect(Object.keys(newest).sort()).toEqual(["at_end", "at_start", "first_seq", "gap", "head_seq", "lines"]);
    expect(newest).toMatchObject({ head_seq: TOTAL, first_seq: 1, at_start: false, at_end: true, gap: false });
    expect(newest.lines).toHaveLength(220);
    expect(newest.lines[219].seq).toBe(TOTAL);
    const mid = await win(LOAD_END_S + 1, { from: 5000, maxLines: 9999 });
    expect(mid.lines).toHaveLength(500);
    expect(mid.lines[0].seq).toBe(5000);
    expect(mid).toMatchObject({ at_start: false, at_end: false });
    const top = await win(LOAD_END_S + 1, { from: 0, maxLines: 10 });
    expect(top).toMatchObject({ at_start: true, at_end: false });
    expect(top.lines[0].seq).toBe(1);
  });

  it("before the load the stream is empty, as the core answers an empty stream", async () => {
    expect(await win(0, { from: null, maxLines: 220 })).toEqual({ lines: [], head_seq: 0, first_seq: 0, at_start: true, at_end: true, gap: false });
  });

  it("has no dead replies for commands the UI no longer sends", async () => {
    const { transport } = fixtureTransport(() => LOAD_END_S + 1);
    expect(await transport.call("read_spill", { offset: 0 })).toEqual({});
  });

  it("answers tail as the core does: the lines after afterSeq, within the byte budget, with the real head", async () => {
    const { transport } = fixtureTransport(() => LOAD_END_S + 1);
    const t = (await transport.call("tail", { stream: "w-a1", afterSeq: TOTAL - 3, maxBytes: 65536 })) as { lines: { seq: number }[]; head_seq: number; gap: boolean };
    expect(t.lines.map((l) => l.seq)).toEqual([TOTAL - 2, TOTAL - 1, TOTAL]);
    expect(t).toMatchObject({ head_seq: TOTAL, gap: false });
    const capped = (await transport.call("tail", { stream: "w-a1", afterSeq: 0, maxBytes: 1 })) as { lines: unknown[]; head_seq: number };
    expect(capped.lines).toHaveLength(1);
    expect(capped.head_seq).toBe(TOTAL);
  });

  it("the perf build opens the worker on its Live tab, which holds a capped feed and no error", async () => {
    window.location.hash = "#/crew?slug=camp&id=r1";
    setTransport(fixtureTransport(() => LOAD_END_S + 1).transport);
    render(<Crew />);
    const log = await screen.findByRole("log", { name: S.crew.live.label });
    await waitFor(() => expect(log.textContent).toContain(String(TOTAL).padStart(6, "0")));
    expect(log.children.length).toBeLessThanOrEqual(300);
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("the perf build's Raw output tab renders rows and no error", async () => {
    window.location.hash = "#/crew?slug=camp&id=r1";
    setTransport(fixtureTransport(() => LOAD_END_S + 1).transport);
    render(<Crew />);
    fireEvent.click(within(await screen.findByRole("tree")).getByText("Backend").closest("[role=treeitem]") as HTMLElement);
    fireEvent.click(await screen.findByRole("tab", { name: S.crew.tabs.stream }));
    const log = await screen.findByRole("log", { name: S.crew.streamLabel });
    await waitFor(() => expect(log.textContent).toContain(String(TOTAL).padStart(6, "0")));
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("is a closed world: unknown commands answer empty, the crew has one worker", async () => {
    const { transport } = fixtureTransport(() => 0);
    expect(await transport.call("anything_else")).toEqual({});
    const c = (await transport.call("crew")) as { children: { children: { id: string }[] }[] };
    expect(c.children[0].children[0].id).toBe("agent:a1");
  });
});
