// FG-06 (gap matrix): Given project tasks and worker attempts, when statuses change, then the board agrees with the
// one authoritative revision; the board is read-only and fully usable from the keyboard (ADR-15 D7, WCAG 2.2 AA).
import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { setTransport } from "../src/ipc";
import { Board } from "../src/screens/board/Board";
import { CurrentProject } from "../src/projects/current";
import type { Current } from "../src/projects/current";
import { S } from "../strings";
import { fakeIpc } from "./fakeIpc";

const task = (local: string, status: string, title: string) => ({ id: `r1.${local}`, local, title, owner: "bhima", status, deps: [], wave: 1 });
const overviewOf = (tasks: ReturnType<typeof task>[]) => ({
  slug: "c1", run_id: "r1", stale: false,
  timeline: { phases: [], current: "execute", status: "active" },
  graph: { waves: 1, tasks },
});
const current: Current = { project: { schema: "vajra.project/1", id: "p_1", name: "api", root: "/src/api", created: null, archived: null, rev: 0 }, campaigns: [], focus: "c1", runId: "r1" };
const state = (rev: number) => ({ schema: "vajra.project_state/1", project_id: "p_1", rev, unchanged: false, active_runs: 1, generations: { project: 0 }, workers: [], receipts: [] });
const changed = (rev: number, id = "p_1") => ({ streams: ["workers"], snapshot_needed: false, projects: [{ project_id: id, rev }] });
const inboxOf = (items: unknown[]) => ({ count: items.length, items });
const question = { id: "i1", kind: "plan", campaign: "c1", origin: "app", created: "", age_s: 3, title: "Deploy to staging", body: "", actions: [] };

let tasks = [task("t1", "in_progress", "Token bucket"), task("t2", "pending", "Docs"), task("t3", "pending", "Metrics")];
let rev = 10;
let ipc: ReturnType<typeof fakeIpc>;
const mount = () => {
  ipc = fakeIpc({
    project_state: () => state(rev),
    run_overview: () => overviewOf(tasks),
    evidence_list: { slug: "c1", stale: false, rows: [] },
    inbox: inboxOf([question]),
  });
  setTransport(ipc.transport);
  return render(<CurrentProject.Provider value={current}><Board /></CurrentProject.Provider>);
};
const n = (cmd: string) => ipc.calls.filter((c) => c.cmd === cmd).length;
const settle = () => act(async () => { await vi.advanceTimersByTimeAsync(0); });
const tick = (ms: number) => act(async () => { await vi.advanceTimersByTimeAsync(ms); });
const lane = (name: string) => screen.getByRole("region", { name: new RegExp(`^${name},`) });

beforeEach(() => { vi.useFakeTimers(); tasks = [task("t1", "in_progress", "Token bucket"), task("t2", "pending", "Docs"), task("t3", "pending", "Metrics")]; rev = 10; });
afterEach(() => { cleanup(); setTransport(); vi.useRealTimers(); });

describe("Board follows the project revision", () => {
  it("does not reload for an event that leaves the revision where it was", async () => {
    mount(); await settle(); await tick(600);
    expect(n("run_overview")).toBe(1);
    act(() => ipc.emit("changed", changed(10)));
    act(() => ipc.emit("changed", changed(50, "p_other")));
    await tick(1200);
    expect(n("run_overview")).toBe(1);
    expect(n("evidence_list")).toBe(1);
  });

  it("reloads once when the revision moves, and the card changes lane", async () => {
    mount(); await settle(); await tick(600);
    expect(within(lane(S.board.lanes.running)).getByText("Token bucket")).toBeTruthy();
    tasks = [task("t1", "done", "Token bucket"), task("t2", "pending", "Docs"), task("t3", "pending", "Metrics")];
    rev = 14;
    for (let r = 11; r <= 14; r++) act(() => ipc.emit("changed", changed(r)));
    await settle(); await tick(50);
    expect(n("run_overview")).toBe(2);
    // Done only when every check passes; with no evidence it waits in Review, never Done.
    expect(within(lane(S.board.lanes.review)).getByText("Token bucket")).toBeTruthy();
    expect(within(lane(S.board.lanes.running)).queryByText("Token bucket")).toBeNull();
  });

  it("falls back to raw change events when the core cannot give a revision", async () => {
    ipc = fakeIpc({ run_overview: () => overviewOf(tasks), evidence_list: { slug: "c1", stale: false, rows: [] }, inbox: inboxOf([]) });
    setTransport(ipc.transport);
    render(<CurrentProject.Provider value={current}><Board /></CurrentProject.Provider>);
    await settle(); await tick(600);
    act(() => ipc.emit("changed", { streams: ["workers"], snapshot_needed: false }));
    await settle(); await tick(50);
    expect(n("run_overview")).toBe(2);
  });
});

describe("Board keyboard (D7: focus moves, cards never do)", () => {
  const cards = () => screen.getAllByRole("button").filter((b) => b.classList.contains("v-card"));
  const titles = () => cards().map((b) => b.textContent);
  const ready = async () => { mount(); await settle(); await tick(50); };

  it("keeps exactly one card in the tab order", async () => {
    await ready();
    expect(cards().filter((b) => b.tabIndex === 0)).toHaveLength(1);
  });

  it("arrow keys move focus within a lane and across lanes, and Home/End jump", async () => {
    await ready();
    const docs = screen.getByText("Docs").closest("button")!;
    docs.focus();
    fireEvent.keyDown(docs, { key: "ArrowDown" });
    expect(document.activeElement).toBe(screen.getByText("Metrics").closest("button"));
    fireEvent.keyDown(document.activeElement!, { key: "Home" });
    expect(document.activeElement).toBe(docs);
    fireEvent.keyDown(docs, { key: "End" });
    expect(document.activeElement).toBe(screen.getByText("Metrics").closest("button"));
    fireEvent.keyDown(document.activeElement!, { key: "ArrowLeft" });
    expect(document.activeElement).toBe(screen.getByText("Metrics").closest("button")); // Backlog is the first lane: nowhere left to go
    fireEvent.keyDown(docs, { key: "ArrowRight" });
    expect(lane(S.board.lanes.running).contains(document.activeElement)).toBe(true);
  });

  it("never moves a card or changes a lane's contents", async () => {
    await ready();
    const before = titles();
    const first = cards()[0];
    first.focus();
    for (const key of ["ArrowDown", "ArrowRight", "ArrowUp", "ArrowLeft", "Home", "End", " ", "Escape"]) fireEvent.keyDown(document.activeElement ?? first, { key });
    expect(titles()).toEqual(before);
  });

  it("makes the focused card the one in the tab order", async () => {
    await ready();
    const docs = screen.getByText("Docs").closest("button")!;
    docs.focus();
    fireEvent.keyDown(docs, { key: "ArrowDown" });
    const metrics = screen.getByText("Metrics").closest("button")!;
    expect(metrics.tabIndex).toBe(0);
    expect(docs.tabIndex).toBe(-1);
  });
});

describe("Board accessibility (WCAG 2.2 AA)", () => {
  const ready = async () => { mount(); await settle(); await tick(50); };

  it("names a waiting card by everything it shows, so the label contains the visible text", async () => {
    await ready();
    const plan = screen.getByText("Deploy to staging").closest("button")!;
    const name = plan.textContent ?? "";
    expect(name).toContain(S.board.needsYou);
    expect(name).toContain("Deploy to staging");
    expect(plan.getAttribute("aria-label")).toBeNull();
  });

  it("tells a screen reader how many cards need you, politely, and nothing when none do", async () => {
    await ready();
    const live = screen.getByRole("status");
    expect(live.textContent).toBe(S.frame.needsYou(1));
    expect(live.getAttribute("aria-live") ?? "polite").toBe("polite");
  });

  it("offers no drag handle and says the board is read-only", async () => {
    await ready();
    expect(document.querySelector("[draggable]")).toBeNull();
    expect(screen.getByText(S.board.readOnly)).toBeTruthy();
  });

  it("gives each lane a heading a screen reader can reach", async () => {
    await ready();
    expect(screen.getAllByRole("heading", { level: 2 })).toHaveLength(4);
  });
});
