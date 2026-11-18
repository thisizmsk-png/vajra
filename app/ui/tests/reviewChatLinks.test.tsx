// The Review page's task rows open their worker's chat and say what a working one is doing; an old run that still says
// running reads Stalled there and in the sessions list, by the Workers screen's own rule.
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { setTransport } from "../src/ipc";
import type { CrewNodeDto } from "../src/ipc";
import { sessionsByDay } from "../src/projects/adapter";
import { ProjectsProvider } from "../src/projects/store";
import { STALL_MS, runStalled } from "../src/screens/crew/crewLive";
import { Run } from "../src/screens/run/Run";
import { latestWorkers } from "../src/screens/run/taskActivity";
import { S } from "../strings";
import { fakeIpc } from "./fakeIpc";

afterEach(() => {
  cleanup();
  setTransport();
  window.location.hash = "";
});

const now = Date.parse("2026-11-17T12:00:00Z");
const DAY = 24 * 3600_000;

describe("the stall rule outside the Workers screen", () => {
  it("is stalled only when it says running, no process here runs it, and its record has not moved lately", () => {
    expect(runStalled({ running: true, live: false, lastMs: now - 3 * DAY, now })).toBe(true);
    expect(runStalled({ running: true, live: false, lastMs: null, now })).toBe(true);
    // A run started elsewhere that is still writing its record is not called stalled.
    expect(runStalled({ running: true, live: false, lastMs: now - STALL_MS / 2, now })).toBe(false);
    expect(runStalled({ running: true, live: true, lastMs: now - 3 * DAY, now })).toBe(false);
    expect(runStalled({ running: false, live: false, lastMs: now - 3 * DAY, now })).toBe(false);
    // Nothing is claimed when the app's live runs could not be read.
    expect(runStalled({ running: true, live: null, lastMs: now - 3 * DAY, now })).toBe(false);
  });
});

describe("the sessions list", () => {
  const camp = (slug: string, updated: string, status = "active") => ({ slug, goal: slug, status, created: updated, updated });

  it("an old run that still says running reads Stalled, with a blocked dot and its own count", () => {
    const days = sessionsByDay([camp("old", "2026-11-14 09:00"), camp("live", "2026-11-14 10:00"), camp("done", "2026-11-14 11:00", "done")], new Date(now), new Set(["live"]));
    const rows = days.flatMap((d) => d.items);
    expect(rows.find((r) => r.slug === "old")).toMatchObject({ status: "running", stalled: true, dot: "blocked" });
    expect(rows.find((r) => r.slug === "live")).toMatchObject({ dot: "in_progress" });
    expect(rows.find((r) => r.slug === "live")?.stalled).toBeUndefined();
    expect(days[0].summary).toBe("1 running, 1 stalled, 1 done");
  });

  it("with no word on live runs, nothing reads Stalled", () => {
    const rows = sessionsByDay([camp("old", "2026-11-14 09:00")], new Date(now)).flatMap((d) => d.items);
    expect(rows[0].stalled).toBeUndefined();
  });
});

const node = (id: string, kind: string, title: string, status: string, children: CrewNodeDto[] = []): CrewNodeDto =>
  ({ id, kind, title, engine_id: null, status, overlays: [], children });
const crew = node("lead", "lead", "Lead", "in_progress", [node("run:r1", "run", "Run r1", "in_progress", [
  node("task:t1", "task", "t1", "done", [node("agent:a1", "round", "round 1", "done")]),
  node("task:t2", "task", "t2", "in_progress", [node("agent:old", "round", "round 1", "failed"), node("agent:a2", "round", "round 2", "in_progress")]),
])]);

describe("task rows", () => {
  it("map a task to its latest round's worker", () => {
    expect([...latestWorkers(crew)]).toEqual([["t1", "a1"], ["t2", "a2"]]);
  });

  const phases = ["Understand", "Plan", "Execute", "Verify"].map((name, i) => ({ name, state: i < 2 ? "done" : i === 2 ? "current" : "pending" }));
  const gtask = (local: string, status: string) => ({ id: `r1.${local}`, local, title: `Title ${local}`, owner: "Backend", status, deps: [], wave: 0 });
  const overview = { slug: "camp", run_id: "20261117-1100", stale: false, timeline: { phases, current: "Execute", status: "active" }, graph: { tasks: [gtask("t1", "closed"), gtask("t2", "in_progress")], waves: 1 } };
  const open = (live: boolean, updated: string, state: { live: boolean; ov: typeof overview } = { live, ov: overview }) => {
    window.location.hash = "#/run?slug=camp";
    const f = fakeIpc({
      list_campaigns: () => ({ campaigns: [{ slug: "camp", goal: "Rate limits", status: state.ov.timeline.status, updated }] }), resume_targets: { targets: {} },
      active_runs: () => ({ count: state.live ? 1 : 0, runs: state.live ? [{ run_id: overview.run_id, campaign: "camp" }] : [] }), inbox: { count: 0, items: [] },
      evidence_list: { slug: "camp", stale: false, rows: [] }, run_overview: () => state.ov, crew,
      stream_window: (a?: Record<string, unknown>) => ({ lines: a?.stream === "w-a2" ? [{ seq: 1, text: "[tool] Bash: npm test -- bucket" }] : [], head_seq: 1, first_seq: 1, at_start: true, at_end: true, gap: false }),
      run_log_since: { run_id: overview.run_id, entries: [], head_seq: 0, broken: null, io_error: null },
      evidence_approval: { run_id: overview.run_id, state: "none" },
    });
    setTransport(f.transport);
    render(<ProjectsProvider><Run /></ProjectsProvider>);
    return f;
  };

  it("a working task shows its worker's newest line, muted, and its title opens that worker's chat", async () => {
    open(true, new Date().toISOString());
    const list = await screen.findByRole("list", { name: S.run.graph });
    await waitFor(() => expect(within(list).getByText("Bash npm test -- bucket").className).toBe("v-run__doing"));
    fireEvent.click(within(list).getByRole("button", { name: "t2 Title t2" }));
    expect(window.location.hash).toContain("crew");
    expect(window.location.hash).toContain("focus=task%3At2");
    expect(within(list).getByRole("button", { name: S.run.openChat("t2") })).toBeTruthy();
  });

  it("an old run nothing runs reads Stalled, never Working, and reads no worker lines", async () => {
    const f = open(false, "2026-11-10 09:00");
    const list = await screen.findByRole("list", { name: S.run.graph });
    await waitFor(() => expect(within(list).getByText(S.crew.stalled)).toBeTruthy());
    expect(within(list).queryByText(S.status.in_progress)).toBeNull();
    expect(screen.getByText(/Stalled: no activity since/)).toBeTruthy();
    expect(f.calls.some((c) => c.cmd === "stream_window")).toBe(false);
  });

  it("a working task's line is gone once the run ends and the task is done", async () => {
    const state = { live: true, ov: overview };
    const f = open(true, new Date().toISOString(), state);
    const list = await screen.findByRole("list", { name: S.run.graph });
    await waitFor(() => expect(within(list).getByText("Bash npm test -- bucket")).toBeTruthy());
    state.live = false;
    state.ov = { ...overview, timeline: { ...overview.timeline, status: "done" }, graph: { ...overview.graph, tasks: [gtask("t1", "closed"), gtask("t2", "closed")] } };
    act(() => f.emit("changed", {}));
    await waitFor(() => expect(within(list).queryByText("Bash npm test -- bucket")).toBeNull());
  });

  it("an active run still planning, with no task yet, reads Stalled once nothing runs it", async () => {
    const planning = { ...overview, graph: { tasks: [], waves: 0 } };
    open(false, "2026-11-10 09:00", { live: false, ov: planning });
    await waitFor(() => expect(screen.getByText(/Stalled: no activity since/)).toBeTruthy());
  });

  it("reads no worker line while the window is hidden, and reads as soon as it shows", async () => {
    const hidden = vi.spyOn(document, "hidden", "get").mockReturnValue(true);
    const f = open(true, new Date().toISOString());
    await screen.findByRole("list", { name: S.run.graph });
    await new Promise((r) => setTimeout(r, 50));
    // (The projects store reads the crew tree on its own; the rows read no worker stream.)
    expect(f.calls.some((c) => c.cmd === "stream_window")).toBe(false);
    hidden.mockReturnValue(false);
    act(() => { document.dispatchEvent(new Event("visibilitychange")); });
    const list = screen.getByRole("list", { name: S.run.graph });
    await waitFor(() => expect(within(list).getByText("Bash npm test -- bucket")).toBeTruthy());
    hidden.mockRestore();
  });
});
