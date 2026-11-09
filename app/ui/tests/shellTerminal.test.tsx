// The shell mounts the terminal dock in its terminal slot, so the dock (and with it the lifecycle report) is reachable.
import { readFileSync } from "node:fs";
import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { setTransport } from "../src/ipc";
import { Shell } from "../src/Shell";
import { openTerminalHere } from "../src/terminal/TerminalDock";
import { S } from "../strings";
import { FakeXterm, fakeTerm } from "./fakeTerm";

vi.mock("@xterm/xterm", async () => ({ Terminal: (await import("./fakeTerm")).FakeXterm }));
vi.mock("@xterm/addon-fit", () => ({ FitAddon: class { fit() {} } }));

const store = new Map<string, string>();
vi.stubGlobal("localStorage", { getItem: (k: string) => store.get(k) ?? null, setItem: (k: string, v: string) => void store.set(k, v), removeItem: (k: string) => void store.delete(k), clear: () => store.clear() });

beforeEach(() => {
  store.clear();
  FakeXterm.all = [];
  window.location.hash = "";
  (globalThis as { ResizeObserver?: unknown }).ResizeObserver = class { observe() {} disconnect() {} };
});
afterEach(() => {
  cleanup();
  setTransport();
});

describe("Shell mounts the terminal dock", () => {
  it("puts the dock in the terminal slot, and opening a terminal shows it", async () => {
    const f = fakeTerm({ list_campaigns: { campaigns: [] }, resume_targets: { targets: {} }, active_runs: { count: 0, runs: [] }, inbox: { count: 0, items: [] }, preflight: { rows: [], start_enabled: false } });
    setTransport(f.transport);
    const { container } = render(<Shell />);
    const slot = container.querySelector('[data-slot="terminal"]') as HTMLElement;
    expect(slot).toBeTruthy();
    expect(slot.hidden).toBe(false);
    expect(screen.queryByRole("region", { name: S.terminal.region })).toBeNull(); // nothing until a terminal is opened
    act(() => openTerminalHere({ cwd: "/work/app", label: "app" }));
    const dock = await screen.findByRole("region", { name: S.terminal.region });
    expect(slot.contains(dock)).toBe(true);
    expect(dock.closest(".v-center")).toBeTruthy(); // in the centre column: its width follows the column, with or without the sidebar
    await waitFor(() => expect(f.named("term_open").length).toBe(1));
  });

  it("lays the dock out in its slot: the stylesheet does not pin it to the window", () => {
    const css = readFileSync(`${__dirname}/../src/terminal/terminal.css`, "utf8");
    const rule = /^\.v-term\s*\{([^}]*)\}/m.exec(css)?.[1] ?? "";
    expect(rule).not.toBe("");
    expect(rule).not.toMatch(/position\s*:\s*fixed/);
    expect(rule).not.toMatch(/--sidebar-w/);
    expect(rule).toMatch(/flex\s*:\s*none/);
  });
});
