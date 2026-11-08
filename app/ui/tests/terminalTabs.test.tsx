// FG-16 gap matrix: a test shell and an agent terminal live in separate tabs, and the tab layout survives a restart.
// A restart keeps the layout (folders, kinds, order, active tab), never the shells: those die with the app (ADR-9).
import { cleanup, fireEvent, render, screen, waitFor, act } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { setTransport } from "../src/ipc";
import { LAYOUT_KEY, TerminalDock, openTerminalHere } from "../src/terminal/TerminalDock";
import { FakeXterm, fakeTerm } from "./fakeTerm";

vi.mock("@xterm/xterm", async () => ({ Terminal: (await import("./fakeTerm")).FakeXterm }));
vi.mock("@xterm/addon-fit", () => ({ FitAddon: class { fit() {} } }));

// Node's own experimental localStorage shadows jsdom's here, so give the dock a plain in-memory one.
const store = new Map<string, string>();
const memory = { getItem: (k: string) => store.get(k) ?? null, setItem: (k: string, v: string) => void store.set(k, v), removeItem: (k: string) => void store.delete(k), clear: () => store.clear() };
vi.stubGlobal("localStorage", memory);

const APP = { cwd: "/work/app", label: "app" };
const saved = () => JSON.parse(localStorage.getItem(LAYOUT_KEY) ?? "null");
const tabEls = () => screen.queryAllByRole("tab", { hidden: true });
const tabNames = () => tabEls().map((t) => t.textContent);

beforeEach(() => {
  localStorage.clear();
  FakeXterm.all = [];
  (globalThis as { ResizeObserver?: unknown }).ResizeObserver = class { observe() {} disconnect() {} };
});
afterEach(() => {
  cleanup();
  setTransport();
});

function mount(over: Parameters<typeof fakeTerm>[0] = {}) {
  const f = fakeTerm(over);
  setTransport(f.transport);
  const r = render(<TerminalDock defaultPlace={APP} />);
  return { f, ...r };
}

describe("tabs by kind", () => {
  it("GIVEN a shell tab in a folder WHEN an agent terminal opens in the same folder THEN it gets its own tab", async () => {
    const { f } = mount();
    act(() => openTerminalHere(APP));
    await waitFor(() => expect(f.named("term_open").length).toBe(1));
    act(() => openTerminalHere({ ...APP, kind: "agent", label: "worker-1" }));
    await waitFor(() => expect(f.named("term_open").length).toBe(2));
    expect(tabEls()).toHaveLength(2);
  });

  it("GIVEN an agent tab WHEN a shell is asked for in the same folder THEN the shell does not steal the agent's tab", async () => {
    const { f } = mount();
    act(() => openTerminalHere({ ...APP, kind: "agent", label: "worker-1" }));
    await waitFor(() => expect(f.named("term_open").length).toBe(1));
    act(() => openTerminalHere(APP));
    await waitFor(() => expect(f.named("term_open").length).toBe(2));
  });

  it("GIVEN a shell tab WHEN the same folder is asked for as a shell again THEN it focuses that tab and opens nothing", async () => {
    const { f } = mount();
    act(() => openTerminalHere(APP));
    await waitFor(() => expect(f.named("term_open").length).toBe(1));
    act(() => openTerminalHere(APP));
    expect(f.named("term_open").length).toBe(1);
    expect(tabEls()).toHaveLength(1);
  });

  it("GIVEN an agent tab WHEN I read the tab list THEN the kind is in its accessible name, not only its colour", async () => {
    const { f } = mount();
    act(() => openTerminalHere({ ...APP, kind: "agent", label: "worker-1" }));
    await waitFor(() => expect(f.named("term_open").length).toBe(1));
    expect(screen.getByRole("tab", { name: /agent/i, hidden: true })).toBeTruthy();
  });

  it("GIVEN an agent tab is active WHEN I press + THEN the new terminal is a plain shell, never a second agent", async () => {
    const { f } = mount();
    act(() => openTerminalHere({ ...APP, kind: "agent", label: "worker-1" }));
    await waitFor(() => expect(f.named("term_open").length).toBe(1));
    fireEvent.click(screen.getByRole("button", { name: "New terminal", hidden: true }));
    await waitFor(() => expect(f.named("term_open").length).toBe(2));
    expect(saved().tabs.map((t: { kind: string }) => t.kind)).toEqual(["agent", "shell"]);
  });
});

describe("durable layout", () => {
  it("GIVEN open tabs WHEN the layout changes THEN folders, kinds, order and the active tab are saved, and nothing else", async () => {
    const { f } = mount();
    act(() => openTerminalHere(APP));
    act(() => openTerminalHere({ cwd: "/work/app/wt", label: "app/wt", kind: "agent" }));
    await waitFor(() => expect(f.named("term_open").length).toBe(2));
    const s = saved();
    expect(s.tabs).toEqual([
      { cwd: "/work/app", label: "app", kind: "shell" },
      { cwd: "/work/app/wt", label: "app/wt", kind: "agent" },
    ]);
    expect(s.cur).toBe(1);
    expect(Object.keys(s).sort()).toEqual(["cur", "tabs", "v"]); // no scrollback, no ids, no output
  });

  it("GIVEN a saved layout and no live shells WHEN the app starts THEN the tabs come back without starting any shell", async () => {
    localStorage.setItem(LAYOUT_KEY, JSON.stringify({ v: 1, cur: 1, tabs: [{ cwd: "/work/app", label: "app", kind: "shell" }, { cwd: "/work/app/wt", label: "app/wt", kind: "agent" }] }));
    const { f } = mount();
    await waitFor(() => expect(tabNames().length).toBe(2));
    await act(async () => {});
    expect(f.named("term_open")).toHaveLength(0); // no always-on cost: nothing runs until a tab is used
    expect(FakeXterm.all).toHaveLength(0); // and the xterm chunk is not even created
    expect(tabEls()[1].getAttribute("aria-selected")).toBe("true");
  });

  it("GIVEN restored tabs WHEN I open the dock THEN only the active tab starts its shell, in its saved folder", async () => {
    localStorage.setItem(LAYOUT_KEY, JSON.stringify({ v: 1, cur: 1, tabs: [{ cwd: "/work/app", label: "app", kind: "shell" }, { cwd: "/work/other", label: "other", kind: "shell" }] }));
    const { f } = mount();
    await waitFor(() => expect(tabNames().length).toBe(2));
    act(() => void fireEvent.keyDown(window, { key: "`", code: "Backquote", ctrlKey: true }));
    await waitFor(() => expect(f.named("term_open").length).toBe(1));
    expect(f.named("term_open")[0].args?.cwd).toBe("/work/other");
  });

  it("GIVEN restored tabs WHEN I select another tab THEN that one starts too", async () => {
    localStorage.setItem(LAYOUT_KEY, JSON.stringify({ v: 1, cur: 0, tabs: [{ cwd: "/work/app", label: "app", kind: "shell" }, { cwd: "/work/other", label: "other", kind: "shell" }] }));
    const { f } = mount();
    await waitFor(() => expect(tabNames().length).toBe(2));
    act(() => void fireEvent.keyDown(window, { key: "`", code: "Backquote", ctrlKey: true }));
    await waitFor(() => expect(f.named("term_open").length).toBe(1));
    fireEvent.click(tabEls()[1]);
    await waitFor(() => expect(f.named("term_open").length).toBe(2));
  });

  it("GIVEN a restored tab whose folder is gone WHEN it starts THEN it shows the folder error with Try again, and keeps the tab", async () => {
    localStorage.setItem(LAYOUT_KEY, JSON.stringify({ v: 1, cur: 0, tabs: [{ cwd: "/gone", label: "gone", kind: "shell" }] }));
    const { f } = mount({ term_open: () => Object.assign(new Error("x"), { error: "bad_cwd" }) });
    await waitFor(() => expect(tabNames().length).toBe(1));
    act(() => void fireEvent.keyDown(window, { key: "`", code: "Backquote", ctrlKey: true }));
    await waitFor(() => expect(screen.getByRole("alert", { hidden: true })).toBeTruthy());
    expect(f.named("term_open")).toHaveLength(1);
    expect(tabNames()).toHaveLength(1);
  });

  it("GIVEN live shells that outlived the webview AND a saved layout WHEN the dock mounts THEN it attaches to them and restores the rest dormant", async () => {
    localStorage.setItem(LAYOUT_KEY, JSON.stringify({ v: 1, cur: 0, tabs: [{ cwd: "/work/app", label: "app", kind: "agent" }, { cwd: "/work/other", label: "other", kind: "shell" }] }));
    const { f } = mount({ term_list: { terminals: [{ id: 7, cwd: "/work/app", root: "/work/app", rows: 24, cols: 80, alive: true }] } });
    await waitFor(() => expect(f.named("term_attach")[0]?.args).toEqual({ id: 7 }));
    expect(tabNames().length).toBe(2); // the attached one is not duplicated
    expect(tabEls()[0].textContent).toMatch(/agent/i); // and it keeps the kind it was saved with
    expect(f.named("term_open")).toHaveLength(0);
  });

  it("GIVEN a saved tab WHEN I close it THEN it is gone from the saved layout", async () => {
    localStorage.setItem(LAYOUT_KEY, JSON.stringify({ v: 1, cur: 0, tabs: [{ cwd: "/work/app", label: "app", kind: "shell" }] }));
    mount();
    await waitFor(() => expect(tabNames().length).toBe(1));
    fireEvent.click(screen.getByRole("button", { name: "Close app", hidden: true }));
    expect(saved().tabs).toEqual([]);
  });

  it.each([
    ["not json", "{"],
    ["wrong version", JSON.stringify({ v: 2, cur: 0, tabs: [{ cwd: "/a", label: "a", kind: "shell" }] })],
    ["wrong shape", JSON.stringify({ v: 1, cur: 0, tabs: "x" })],
    ["relative folder", JSON.stringify({ v: 1, cur: 0, tabs: [{ cwd: "rel", label: "r", kind: "shell" }] })],
    ["unknown kind", JSON.stringify({ v: 1, cur: 0, tabs: [{ cwd: "/a", label: "a", kind: "root" }] })],
  ])("GIVEN a damaged saved layout (%s) WHEN the dock mounts THEN it starts empty and does not throw", async (_n, raw) => {
    localStorage.setItem(LAYOUT_KEY, raw);
    mount();
    await act(async () => {});
    expect(tabNames()).toHaveLength(0);
  });

  it("GIVEN a saved layout of 500 tabs WHEN the dock mounts THEN it restores at most 16", async () => {
    const tabs = Array.from({ length: 500 }, (_, i) => ({ cwd: `/w/${i}`, label: `w${i}`, kind: "shell" }));
    localStorage.setItem(LAYOUT_KEY, JSON.stringify({ v: 1, cur: 499, tabs }));
    mount();
    await waitFor(() => expect(tabNames().length).toBe(16));
    expect(tabEls().filter((t) => t.getAttribute("aria-selected") === "true")).toHaveLength(1);
  });

  it("GIVEN storage is unavailable WHEN tabs change THEN the dock still works", async () => {
    const spy = vi.spyOn(memory, "setItem").mockImplementation(() => { throw new Error("quota"); });
    const { f } = mount();
    act(() => openTerminalHere(APP));
    await waitFor(() => expect(f.named("term_open").length).toBe(1));
    spy.mockRestore();
  });
});
