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
  runs: [],
  found: [],
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

async function shown(r: unknown, over: Parameters<typeof fakeTerm>[0] = {}) {
  const f = fakeTerm({ lifecycle: r, ...over });
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

  it("GIVEN the dock is open and a run starts then ends with no terminal change WHEN the app says its runs changed THEN the line follows", async () => {
    let r = report({ close_keeps: { terminals: 1, runs: 0 }, quit_ends: { terminals: 1, runs: 0 } });
    const f = await shown(r, { lifecycle: () => r });
    await waitFor(() => expect(line()).toMatch(/1 terminal keeps running/i));
    expect(line()).not.toMatch(/run\b|runs/i);
    r = report({ close_keeps: { terminals: 1, runs: 1 }, quit_ends: { terminals: 1, runs: 1 } });
    act(() => { f.emit("changed", {}); f.emit("changed", {}); }); // a burst is one refresh
    await waitFor(() => expect(line()).toMatch(/1 terminal and 1 run keep running/i));
    const asked = f.named("lifecycle").length;
    r = report({ close_keeps: { terminals: 1, runs: 0 }, quit_ends: { terminals: 1, runs: 0 } });
    act(() => f.emit("changed", {}));
    await waitFor(() => expect(line()).not.toMatch(/run\b|runs/i));
    expect(f.named("lifecycle").length).toBe(asked + 1);
  });

  it("GIVEN the dock is hidden WHEN the app says its runs changed THEN the core is not asked", async () => {
    const f = fakeTerm({ lifecycle: report() });
    setTransport(f.transport);
    render(<TerminalDock defaultPlace={PLACE} />);
    act(() => f.emit("changed", {}));
    await new Promise((r) => setTimeout(r, 400));
    expect(f.named("lifecycle").length).toBe(0);
    expect(f.listenerCount("changed")).toBe(0);
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

const owned = (pid: number, cwd = "/work/app") => ({ pid, stamp: 1, cwd, verdict: "owned", members: 2 });
const reused = (pid: number) => ({ pid, stamp: 1, cwd: "/work/app", verdict: "reused" });
const gone = (pid: number) => ({ pid, stamp: 1, cwd: "/work/app", verdict: "gone" });

describe("after a crash", () => {
  it("GIVEN a shell the last session recorded and that is provably still ours WHEN shown THEN it is listed with a way to end it", async () => {
    await shown(report({ found: [owned(4242, "/work/app/wt")] }));
    const box = await screen.findByTestId("v-term-found");
    expect(box.textContent).toMatch(/4242/);
    expect(box.textContent).toMatch(/\/work\/app\/wt/);
    expect(screen.getByRole("button", { name: /end/i })).toBeTruthy();
  });

  it("GIVEN a pid that now belongs to another program WHEN shown THEN it is named as left alone and offers no kill", async () => {
    await shown(report({ found: [reused(777)] }));
    const box = await screen.findByTestId("v-term-found");
    expect(box.textContent).toMatch(/777/);
    expect(box.textContent).toMatch(/another program|left alone/i);
    expect(screen.queryByRole("button", { name: /end/i })).toBeNull();
  });

  it("GIVEN only gone records WHEN shown THEN nothing is said about them", async () => {
    await shown(report({ found: [gone(5)] }));
    await waitFor(() => expect(screen.getByTestId("v-term-lifecycle")).toBeTruthy());
    expect(screen.queryByTestId("v-term-found")).toBeNull();
  });

  it("GIVEN owned and reused records WHEN I end them THEN only the owned pid is sent, and the report is asked again", async () => {
    const f = await shown(report({ found: [owned(4242), reused(777)] }), { recover_orphans: { results: [{ pid: 4242, result: "ended" }] } });
    const before = f.named("lifecycle").length;
    fireEvent.click(await screen.findByRole("button", { name: /end/i }));
    await waitFor(() => expect(f.named("recover_orphans").length).toBe(1));
    expect(f.named("recover_orphans")[0].args).toEqual({ pids: [4242] });
    await waitFor(() => expect(f.named("lifecycle").length).toBeGreaterThan(before));
  });
});

describe("recovering leftovers honestly", () => {
  it("GIVEN a leftover shell WHEN some of its processes survive the kill THEN the dock says so and does not claim it ended", async () => {
    await shown(report({ found: [owned(4242)] }), { recover_orphans: { results: [{ pid: 4242, result: { survived: { remaining: 1 } } }] } });
    fireEvent.click(await screen.findByRole("button", { name: /end/i }));
    expect(await screen.findByText(/still running/i)).toBeTruthy();
  });

  it("GIVEN survivors WHEN the refreshed report no longer lists the shell THEN the warning stays", async () => {
    let recovered = false;
    await shown(report(), {
      lifecycle: () => report({ found: recovered ? [] : [owned(4242)] }),
      recover_orphans: () => ((recovered = true), { results: [{ pid: 4242, result: { survived: { remaining: 1 } } }] }),
    });
    fireEvent.click(await screen.findByRole("button", { name: /end/i }));
    await waitFor(() => expect(screen.queryByTestId("v-term-found")).toBeNull());
    expect(screen.getByText(/still running/i)).toBeTruthy();
  });

  it("GIVEN a leftover shell WHEN recovery is not possible THEN the dock says so and nothing throws", async () => {
    await shown(report({ found: [owned(4242)] }), { recover_orphans: new Error("boom") });
    fireEvent.click(await screen.findByRole("button", { name: /end/i }));
    expect(await screen.findByText(/could not be ended/i)).toBeTruthy();
  });

  it("GIVEN a clean recovery WHEN it finishes THEN no warning is shown", async () => {
    const f = await shown(report({ found: [owned(4242)] }), { recover_orphans: { results: [{ pid: 4242, result: "ended" }] } });
    fireEvent.click(await screen.findByRole("button", { name: /end/i }));
    await waitFor(() => expect(f.named("recover_orphans").length).toBe(1));
    await act(async () => {});
    expect(screen.queryByText(/still running|could not be ended/i)).toBeNull();
  });
});

describe("run groups and archive", () => {
  it("GIVEN active run groups WHEN I open the details THEN each shows its group leader pid", async () => {
    await shown(report({ runs: [{ stream: "s1", pid: 900 }], close_keeps: { terminals: 1, runs: 1 }, quit_ends: { terminals: 1, runs: 1 } }));
    fireEvent.click(await screen.findByText(/terminal details/i));
    expect(screen.getAllByRole("listitem").some((r) => /pid 900/.test(r.textContent ?? "") && /run/i.test(r.textContent ?? ""))).toBe(true);
  });

  it("GIVEN a run and a worktree shell WHEN shown THEN the archive sentence says the run's group is not part of what stays open", async () => {
    await shown(report({ runs: [{ stream: "s1", pid: 900 }], terminals: [diag(1, { cwd: "/work/app/wt", in_worktree: true })], archive_leaves: [1], close_keeps: { terminals: 1, runs: 1 }, quit_ends: { terminals: 1, runs: 1 } }));
    await waitFor(() => expect(line()).toMatch(/archiv/i));
    expect(line()).toMatch(/does not end/i);
  });
});

describe("diagnostics preview", () => {
  const redacted = "Vajra lifecycle diagnostics (local, redacted)\n  id 1 pid 101 running ~/work/app\n";

  it("GIVEN the details WHEN I ask for a preview THEN the core's redacted text is shown, and says it stays on this computer", async () => {
    const f = fakeTerm({ lifecycle: report(), diagnostics_preview: { text: redacted } });
    setTransport(f.transport);
    render(<TerminalDock defaultPlace={PLACE} />);
    act(() => openTerminalHere(PLACE));
    fireEvent.click(await screen.findByRole("button", { name: /preview diagnostics/i }));
    expect((await screen.findByTestId("v-term-diag")).textContent).toBe(redacted);
    expect(screen.getByText(/stays on this computer/i)).toBeTruthy();
  });

  it("GIVEN a preview WHEN I copy THEN the exact text goes to the clipboard and nothing else is sent", async () => {
    const written: string[] = [];
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText: async (t: string) => void written.push(t) } });
    const f = fakeTerm({ lifecycle: report(), diagnostics_preview: { text: redacted } });
    setTransport(f.transport);
    render(<TerminalDock defaultPlace={PLACE} />);
    act(() => openTerminalHere(PLACE));
    fireEvent.click(await screen.findByRole("button", { name: /preview diagnostics/i }));
    expect((await screen.findByTestId("v-term-diag")).textContent).toContain("~/work/app");
    expect(screen.getByText(/stays on this computer/i)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: /copy/i }));
    await waitFor(() => expect(written).toEqual([redacted]));
    expect(f.calls.map((c) => c.cmd).filter((c) => !/^term_|^lifecycle$|^diagnostics_preview$/.test(c))).toEqual([]);
  });

  it("GIVEN the preview cannot be had WHEN I ask THEN it says so and the dock keeps working", async () => {
    const f = fakeTerm({ lifecycle: report(), diagnostics_preview: new Error("no") });
    setTransport(f.transport);
    render(<TerminalDock defaultPlace={PLACE} />);
    act(() => openTerminalHere(PLACE));
    fireEvent.click(await screen.findByRole("button", { name: /preview diagnostics/i }));
    expect(await screen.findByText(/could not be read/i)).toBeTruthy();
    expect(screen.queryByTestId("v-term-diag")).toBeNull();
  });
});
