// The output budget as the dock uses it (FG-12 S12.5): a flood in a visible pane is coalesced and bounded, a hidden pane
// renders nothing. The queue's own rules are in outputQueue.test.ts.
import { act, cleanup, fireEvent, render, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { setTransport } from "../src/ipc";
import { TerminalDock } from "../src/terminal/TerminalDock";
import { FakeXterm, fakeTerm } from "./fakeTerm";

vi.mock("@xterm/xterm", async () => ({ Terminal: (await import("./fakeTerm")).FakeXterm }));
vi.mock("@xterm/addon-fit", () => ({ FitAddon: class { fit() {} } }));

const store = new Map<string, string>();
vi.stubGlobal("localStorage", { getItem: (k: string) => store.get(k) ?? null, setItem: (k: string, v: string) => void store.set(k, v), removeItem: (k: string) => void store.delete(k), clear: () => store.clear() });

const PLACE = { cwd: "/work/app", label: "app" };
const ctrlTick = () => fireEvent.keyDown(window, { key: "`", code: "Backquote", ctrlKey: true });
const total = (x: FakeXterm) => x.written.reduce((n, w) => n + w.length, 0);

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
  const f = fakeTerm();
  setTransport(f.transport);
  render(<TerminalDock defaultPlace={PLACE} />);
  act(() => void ctrlTick());
  await waitFor(() => expect(f.named("term_open").length).toBe(1));
  await act(async () => {});
  return { f, x: FakeXterm.all[0], out: f.streams[0] };
}

describe("a flood in a visible pane", () => {
  it("GIVEN 160 messages of 64 KiB arriving at once WHEN frames run THEN xterm gets far fewer, bounded writes, a skipped marker, and the core is told about every byte", async () => {
    const { f, x, out } = await opened();
    const MSG = 64 * 1024;
    act(() => {
      for (let i = 0; i < 160; i++) out.send(new Uint8Array(MSG).fill(120).buffer);
    });
    await waitFor(() => expect(x.written.length).toBeGreaterThan(0));
    await waitFor(() => expect(f.named("term_ack").reduce((n, c) => n + (c.args!.bytes as number), 0)).toBeGreaterThanOrEqual(160 * MSG - MSG));
    expect(x.written.every((w) => w.length <= 64 * 1024 + 256)).toBe(true); // a frame's budget (plus a marker write)
    expect(total(x)).toBeLessThan(160 * MSG); // the middle of a 10 MiB flood is not painted
    expect(x.text()).toMatch(/\[skipped \d+ KB of output\]/);
  });

  it("GIVEN ordinary output WHEN it arrives THEN it is written whole with no marker", async () => {
    const { f, x, out } = await opened();
    act(() => out.send(f.bytes("hello world\r\n").slice().buffer as ArrayBuffer));
    await waitFor(() => expect(x.text()).toContain("hello world"));
    expect(x.text()).not.toContain("skipped");
  });
});

describe("a hidden pane", () => {
  it("GIVEN the dock is hidden WHEN output arrives THEN xterm is not written; shown again, the output appears", async () => {
    const { f, x, out } = await opened();
    act(() => void ctrlTick()); // hide
    const before = x.written.length;
    act(() => out.send(f.bytes("while hidden").slice().buffer as ArrayBuffer));
    await act(async () => void (await new Promise((r) => setTimeout(r, 80))));
    expect(x.written.length).toBe(before);
    act(() => void ctrlTick()); // show
    await waitFor(() => expect(x.text()).toContain("while hidden"));
  });

  it("GIVEN the dock is hidden WHEN a flood arrives THEN what is held is bounded and the core is still told (the shell never stalls)", async () => {
    const { f, x, out } = await opened();
    act(() => void ctrlTick()); // hide
    const MSG = 64 * 1024;
    act(() => {
      for (let i = 0; i < 100; i++) out.send(new Uint8Array(MSG).fill(121).buffer);
    });
    // 6.4 MiB sent, at most 256 KiB held: the rest was acknowledged without being rendered
    const acked = f.named("term_ack").reduce((n, c) => n + (c.args!.bytes as number), 0);
    expect(acked).toBeGreaterThanOrEqual(100 * MSG - 256 * 1024 - MSG);
    expect(total(x)).toBe(0);
    act(() => void ctrlTick()); // show
    await waitFor(() => expect(x.text()).toMatch(/skipped/));
  });
});
