// A pane nobody is looking at (FG-12 S12.5, core side): the view tells the core when it is shown or hidden, writes the
// "skipped" line and screen switch the core reports, and the tab shows a mark for output it has not seen.
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { S } from "../strings";
import { setTransport } from "../src/ipc";
import { TerminalDock } from "../src/terminal/TerminalDock";
import { FakeXterm, fakeTerm } from "./fakeTerm";

vi.mock("@xterm/xterm", async () => ({ Terminal: (await import("./fakeTerm")).FakeXterm }));
vi.mock("@xterm/addon-fit", () => ({ FitAddon: class { fit() {} } }));

const store = new Map<string, string>();
vi.stubGlobal("localStorage", { getItem: (k: string) => store.get(k) ?? null, setItem: (k: string, v: string) => void store.set(k, v), removeItem: (k: string) => void store.delete(k), clear: () => store.clear() });

const PLACE = { cwd: "/work/app", label: "app" };
const ctrlTick = () => fireEvent.keyDown(window, { key: "`", code: "Backquote", ctrlKey: true });

beforeEach(() => {
  store.clear();
  FakeXterm.all = [];
  (globalThis as { ResizeObserver?: unknown }).ResizeObserver = class { observe() {} disconnect() {} };
});
afterEach(() => {
  cleanup();
  setTransport();
});

async function opened() {
  const f = fakeTerm({ term_visible: {} });
  setTransport(f.transport);
  render(<TerminalDock defaultPlace={PLACE} />);
  act(() => void ctrlTick());
  await waitFor(() => expect(f.named("term_open").length).toBe(1));
  await act(async () => {});
  return { f, x: FakeXterm.all[0], out: f.streams[0] };
}
const visibleCalls = (f: ReturnType<typeof fakeTerm>) => f.named("term_visible").map((c) => [c.args!.id, c.args!.visible, c.args!.alt]);
const acked = (f: ReturnType<typeof fakeTerm>) => f.named("term_ack").reduce((n, c) => n + (c.args!.bytes as number), 0);

describe("telling the core", () => {
  it("GIVEN a shown pane WHEN the dock is hidden and shown THEN the core is told each time, once", async () => {
    const { f } = await opened();
    expect(visibleCalls(f)).toEqual([]); // it started visible: nothing to say
    act(() => void ctrlTick());
    await waitFor(() => expect(visibleCalls(f)).toEqual([[1, false, false]]));
    act(() => void ctrlTick());
    await waitFor(() => expect(visibleCalls(f)).toEqual([[1, false, false], [1, true, false]]));
  });

  it("GIVEN two tabs WHEN the other tab is selected THEN the first is hidden to the core and the second shown", async () => {
    const { f } = await opened();
    fireEvent.click(screen.getByRole("button", { name: S.terminal.newTab }));
    await waitFor(() => expect(f.named("term_open").length).toBe(2));
    await waitFor(() => expect(visibleCalls(f)).toContainEqual([1, false, false]));
    fireEvent.click(screen.getAllByRole("tab")[0]);
    await waitFor(() => expect(visibleCalls(f)).toContainEqual([1, true, false]));
    expect(visibleCalls(f)).toContainEqual([2, false, false]);
  });

  it("GIVEN a full-screen program (alternate screen) WHEN the pane is hidden THEN the core is told", async () => {
    const { f, x } = await opened();
    x.buffer.active.type = "alternate";
    act(() => void ctrlTick());
    await waitFor(() => expect(visibleCalls(f)).toEqual([[1, false, true]]));
  });
});

describe("what the core reports on show", () => {
  it("GIVEN bytes were left out WHEN the notice and the tail arrive THEN the skipped line comes first and only the core's bytes are acknowledged", async () => {
    const { f, x, out } = await opened();
    act(() => out.send({ skipped: 5 * 1024 * 1024, alt: null }));
    act(() => out.send(f.bytes("the tail\r\n").slice().buffer as ArrayBuffer));
    await waitFor(() => expect(x.text()).toContain("the tail"));
    const t = x.text();
    expect(t).toContain(S.terminal.skipped(5120));
    expect(t.indexOf("skipped")).toBeLessThan(t.indexOf("the tail"));
    await act(async () => {});
    expect(acked(f)).toBeLessThanOrEqual("the tail\r\n".length);
  });

  it("GIVEN the program entered the alternate screen while hidden WHEN shown THEN the view switches screens and writes no skipped line", async () => {
    const { x, out } = await opened();
    act(() => out.send({ skipped: 0, alt: true }));
    await waitFor(() => expect(x.text()).toContain("\x1b[?1049h"));
    expect(x.text()).not.toContain("skipped");
  });

  it("GIVEN the program left the alternate screen while hidden WHEN shown THEN the view leaves it before the skipped line", async () => {
    const { x, out } = await opened();
    act(() => out.send({ skipped: 2048, alt: false }));
    await waitFor(() => expect(x.text()).toContain("skipped"));
    expect(x.text().indexOf("\x1b[?1049l")).toBeGreaterThanOrEqual(0);
    expect(x.text().indexOf("\x1b[?1049l")).toBeLessThan(x.text().indexOf("skipped"));
  });
});

describe("the unread mark", () => {
  it("GIVEN output in a tab that is not in view WHEN it is reported THEN the tab is marked and selecting it clears the mark; the tab in view is never marked", async () => {
    const { f, out } = await opened();
    act(() => out.send({ activity: true }));
    await act(async () => {});
    expect(screen.queryByRole("tab", { name: /new output/ })).toBeNull(); // in view: nothing to flag
    fireEvent.click(screen.getByRole("button", { name: S.terminal.newTab }));
    await waitFor(() => expect(f.named("term_open").length).toBe(2));
    await act(async () => {});
    act(() => out.send({ activity: true })); // the first tab is now behind the second
    const marked = await screen.findByRole("tab", { name: S.terminal.unreadTab("app") });
    expect(marked.querySelector(".v-term__dot")).not.toBeNull();
    fireEvent.click(marked);
    await waitFor(() => expect(screen.queryByRole("tab", { name: /new output/ })).toBeNull());
  });

  it("GIVEN the dock is hidden WHEN output is reported THEN the selected tab is marked until the dock is shown", async () => {
    const { out } = await opened();
    act(() => void ctrlTick()); // hide
    act(() => out.send({ activity: true }));
    expect(await screen.findByRole("tab", { name: S.terminal.unreadTab("app"), hidden: true })).toBeTruthy();
    act(() => void ctrlTick()); // show
    await waitFor(() => expect(screen.queryByRole("tab", { name: /new output/ })).toBeNull());
  });
});
