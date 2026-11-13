// Nothing runs at rest (competitor-ux-specs section 2): with no live run the app makes no calls to the core on a timer,
// and while the window is hidden it makes none even for a live run. Events still reach the screens. Counted over 60 s.
import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { setTransport } from "../src/ipc";
import { Shell } from "../src/Shell";
import { fakeIpc } from "./fakeIpc";

const node = (id: string, kind: string, title: string, status: string, children: unknown[] = [], role: string | null = null) =>
  ({ id, kind, title, engine_id: null, role, status, overlays: [], children });
const crew = node("lead", "lead", "Lead", "done", [
  node("run:r1", "run", "Add rate limits", "done", [
    node("task:t2", "task", "t2 Token bucket", "done", [node("agent:backend", "worker", "Backend", "done", [], "Backend")]),
  ]),
]);
const LIVE = { count: 1, runs: [{ run_id: "r1", campaign: "api-1", stream: "s", session_id: "s" }] };
const IDLE = { count: 0, runs: [] };
const replies = (active: unknown) => ({
  preflight: { rows: [], start_enabled: false },
  list_campaigns: { campaigns: [{ slug: "api-1", goal: "Add rate limits", status: "active", passing: 1, total: 5, created: "2026-11-01 09:00", updated: "2026-11-08 09:00" }] },
  resume_targets: { targets: { "api-1": { session_id: "s", run_id: "r1", cwd: "/src/api" } } },
  active_runs: active,
  inbox: { count: 0, items: [] },
  project_list: { projects: [] },
  crew,
  stream_window: { lines: [], first_seq: 1, head_seq: 0, at_start: true, at_end: true, gap: false },
});

let hidden = false;
const setHidden = (h: boolean) => {
  hidden = h;
  act(() => { document.dispatchEvent(new Event("visibilitychange")); });
};
const wait = (ms: number) => act(async () => { await vi.advanceTimersByTimeAsync(ms); });

/** Open the crew view on run r1 with its worker selected (tree, task and stream all mounted), and let the first reads land. */
async function boot(active: unknown) {
  window.location.hash = "#/crew?slug=api-1&id=r1&worker=backend";
  const ipc = fakeIpc(replies(active));
  setTransport(ipc.transport);
  render(<Shell />);
  await act(async () => { await vi.advanceTimersByTimeAsync(0); });
  await screen.findAllByText("Backend");
  // Debounced follow-ups (a 500 ms event gap, a 0 ms reconcile) and any start-up reads settle first.
  await wait(3000);
  return ipc;
}

beforeEach(() => {
  vi.useFakeTimers({ shouldAdvanceTime: true });
  hidden = false;
  Object.defineProperty(document, "hidden", { configurable: true, get: () => hidden });
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
  window.location.hash = "";
  hidden = false;
});

describe("nothing runs at rest", () => {
  it("makes no calls to the core in 60 s idle when no run is live", async () => {
    const ipc = await boot(IDLE);
    const before = ipc.calls.length;
    await wait(60_000);
    expect(ipc.calls.slice(before).map((c) => c.cmd)).toEqual([]);
  });

  it("makes no calls in 60 s while the window is hidden, even with a run live; showing it reads once at once", async () => {
    const ipc = await boot(LIVE);
    setHidden(true);
    const before = ipc.calls.length;
    await wait(60_000);
    expect(ipc.calls.slice(before).map((c) => c.cmd)).toEqual([]);
    setHidden(false);
    await wait(0);
    const shown = ipc.calls.slice(before).map((c) => c.cmd);
    expect(shown).toContain("crew");
    expect(shown).toContain("stream_window");
  });

  it("does poll a live run while the window shows (the counter above is not blind)", async () => {
    const ipc = await boot(LIVE);
    const before = ipc.calls.length;
    await wait(10_000);
    const polled = ipc.calls.slice(before).map((c) => c.cmd);
    expect(polled.filter((c) => c === "crew").length).toBeGreaterThanOrEqual(4);
    expect(polled.filter((c) => c === "stream_window").length).toBeGreaterThanOrEqual(8);
  });

  it("still follows an event at rest: a `changed` burst is read, then the app is quiet again", async () => {
    const ipc = await boot(IDLE);
    const before = ipc.calls.length;
    act(() => ipc.emit("changed", {}));
    await wait(1000);
    const after = ipc.calls.length;
    expect(after).toBeGreaterThan(before);
    await wait(60_000);
    expect(ipc.calls.length).toBe(after);
  });
});
