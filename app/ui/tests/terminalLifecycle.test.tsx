// FG-24: the dock says, truthfully and from the core's own report, what keeps running after close, archive or quit.
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { setTransport } from "../src/ipc";
import { TerminalDock, openTerminalHere } from "../src/terminal/TerminalDock";
import { FakeXterm, fakeTerm } from "./fakeTerm";

vi.mock("@xterm/xterm", async () => ({ Terminal: (await import("./fakeTerm")).FakeXterm }));
vi.mock("@xterm/addon-fit", () => ({ FitAddon: class { fit() {} } }));

const store = new Map<string, string>();
vi.stubGlobal("localStorage", { getItem: (k: string) => store.get(k) ?? null, setItem: (k: string, v: string) => void store.set(k, v), removeItem: (k: string) => void store.delete(k), clear: () => store.clear() });

const diag = (id: number, over = {}) => ({ id, pid: 100 + id, cwd: "/work/app", root: "/work/app", alive: true, in_worktree: false, ...over });
const report = (over: Record<string, unknown> = {}) => ({
  terminals: [diag(1)],
  close_keeps: { terminals: 1, runs: 0 },
  quit_ends: { terminals: 1, runs: 0 },
  archive_leaves: [],
  ...over,
});
const PLACE = { cwd: "/work/app", label: "app" };

beforeEach(() => {
  store.clear();
  FakeXterm.all = [];
  (globalThis as { ResizeObserver?: unknown }).ResizeObserver = class { observe() {} disconnect() {} };
});
afterEach(() => {
  cleanup();
  setTransport();
});

async function shown(r: unknown) {
  const f = fakeTerm({ lifecycle: r });
  setTransport(f.transport);
  render(<TerminalDock defaultPlace={PLACE} />);
  act(() => openTerminalHere(PLACE));
  await waitFor(() => expect(f.named("term_open").length).toBe(1));
  return f;
}
const line = () => screen.getByTestId("v-term-lifecycle").textContent ?? "";

describe("lifecycle line", () => {
  it("GIVEN a live shell and a run WHEN the dock is shown THEN it says both keep going on close and both end on quit", async () => {
    await shown(report({ close_keeps: { terminals: 1, runs: 1 }, quit_ends: { terminals: 1, runs: 1 } }));
    await waitFor(() => expect(line()).toMatch(/close the window/i));
    expect(line()).toMatch(/1 terminal and 1 run keep running/i);
    expect(line()).toMatch(/quit.*end/i);
  });

  it("GIVEN several shells and no runs WHEN shown THEN it counts shells only, with plurals", async () => {
    await shown(report({ close_keeps: { terminals: 3, runs: 0 }, quit_ends: { terminals: 3, runs: 0 } }));
    await waitFor(() => expect(line()).toMatch(/3 terminals keep running/i));
    expect(line()).not.toMatch(/run\b|runs/i);
  });

  it("GIVEN nothing would keep running or end WHEN shown THEN there is no line at all", async () => {
    const f = await shown(report({ close_keeps: { terminals: 0, runs: 0 }, quit_ends: { terminals: 0, runs: 0 }, terminals: [] }));
    await waitFor(() => expect(f.named("lifecycle").length).toBeGreaterThan(0));
    expect(screen.queryByTestId("v-term-lifecycle")).toBeNull();
  });

  it("GIVEN a live shell in a worker worktree WHEN shown THEN it says archiving the worker leaves that shell open, by tab name", async () => {
    await shown(report({ terminals: [diag(1, { cwd: "/work/app/wt", in_worktree: true })], archive_leaves: [1] }));
    await waitFor(() => expect(line()).toMatch(/archiv/i));
    expect(line()).toMatch(/does not end|stays open/i);
  });

  it("GIVEN no worktree shells WHEN shown THEN it does not mention archiving", async () => {
    await shown(report());
    await waitFor(() => expect(line()).toMatch(/close the window/i));
    expect(line()).not.toMatch(/archiv/i);
  });

  it("GIVEN the report cannot be had WHEN shown THEN the dock still works and makes no claim", async () => {
    const f = fakeTerm({ lifecycle: new Error("boom") });
    setTransport(f.transport);
    render(<TerminalDock defaultPlace={PLACE} />);
    act(() => openTerminalHere(PLACE));
    await waitFor(() => expect(f.named("term_open").length).toBe(1));
    await act(async () => {});
    expect(screen.queryByTestId("v-term-lifecycle")).toBeNull();
  });

  it("GIVEN the dock is hidden WHEN time passes THEN nothing is asked (no polling, no always-on cost)", async () => {
    const f = fakeTerm({ lifecycle: report() });
    setTransport(f.transport);
    render(<TerminalDock defaultPlace={PLACE} />);
    await act(async () => {});
    expect(f.named("lifecycle")).toHaveLength(0);
  });

  it("GIVEN the dock is shown WHEN a tab is added THEN the report is asked again", async () => {
    const f = await shown(report());
    await act(async () => {});
    const before = f.named("lifecycle").length;
    expect(before).toBeGreaterThan(0);
    fireEvent.click(screen.getByRole("button", { name: "New terminal" }));
    await waitFor(() => expect(f.named("lifecycle").length).toBeGreaterThan(before));
  });
});

describe("diagnostics list", () => {
  it("GIVEN live and ended shells WHEN I open the details THEN each shows its folder, pid and state", async () => {
    await shown(report({ terminals: [diag(1), diag(2, { alive: false, cwd: "/work/app/wt", in_worktree: true })] }));
    const d = await screen.findByText(/terminal details/i);
    fireEvent.click(d);
    const rows = screen.getAllByRole("listitem").map((r) => r.textContent ?? "");
    expect(rows.some((t) => t.includes("/work/app") && t.includes("101") && /running/i.test(t))).toBe(true);
    expect(rows.some((t) => t.includes("/work/app/wt") && t.includes("102") && /ended/i.test(t))).toBe(true);
  });
});
