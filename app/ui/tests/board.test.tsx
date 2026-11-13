// FG-06 (ADR-15 §4.2, D6 A, D7): one campaign's board, 4 lanes plus a needs-you badge, read-only.
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { setTransport } from "../src/ipc";
import { Board } from "../src/screens/board/Board";
import { CurrentProject } from "../src/projects/current";
import { ProjectsProvider } from "../src/projects/store";
import type { Current } from "../src/projects/current";
import { S } from "../strings";
import { fakeIpc } from "./fakeIpc";

const task = (local: string, status: string, title: string) => ({ id: `r1.${local}`, local, title, owner: "bhima", status, deps: [], wave: 1 });
const overview = {
  slug: "c1", run_id: "r1", stale: false,
  timeline: { phases: [], current: "execute", status: "active" },
  graph: { waves: 1, tasks: [task("t1", "done", "Design the limiter"), task("t2", "in_progress", "Token bucket"), task("t3", "question", "Retry-After"), task("t4", "pending", "Docs"), task("t5", "done", "Dashboard")] },
};
const ev = (t: string, status: string, criterion: string) => ({ task: t, title: null, owner: null, criterion, status, head: null, path: null, commands: null, report: null });
const current: Current = { project: { schema: "vajra.project/1", id: "p_1", name: "api", root: "/src/api", created: null, archived: null, rev: 0 }, campaigns: [], focus: "c1", runId: "r1" };

const mount = (replies: Record<string, unknown>, cur: Current = current) => {
  setTransport(fakeIpc(replies).transport);
  return render(<CurrentProject.Provider value={cur}><Board /></CurrentProject.Provider>);
};
const ok = {
  run_overview: overview,
  evidence_list: { slug: "c1", stale: false, rows: [ev("r1.t1", "pass", "AC-1"), ev("r1.t5", "pass", "AC-2"), ev("r1.t5", "fail", "AC-3")] },
  inbox: { count: 1, items: [{ id: "i1", kind: "plan", campaign: "c1", origin: "app", created: "", age_s: 3, title: "Deploy to staging", body: "", actions: [] }] },
};
const lane = (name: string) => screen.getByRole("region", { name: new RegExp(`^${name},`) });

afterEach(() => { cleanup(); setTransport(); });

describe("Board", () => {
  it("shows the four lanes with each card where its work and checks put it", async () => {
    mount(ok);
    await screen.findByText("Token bucket");
    expect(within(lane(S.board.lanes.backlog)).getByText("Docs")).toBeTruthy();
    expect(within(lane(S.board.lanes.running)).getByText("Token bucket")).toBeTruthy();
    expect(within(lane(S.board.lanes.running)).getByText("Retry-After")).toBeTruthy();
    expect(within(lane(S.board.lanes.done)).getByText("Design the limiter")).toBeTruthy();
    // Dashboard says done but one criterion fails: it waits in Review, never Done.
    expect(within(lane(S.board.lanes.review)).getByText("Dashboard")).toBeTruthy();
    expect(within(lane(S.board.lanes.review)).getByText("Deploy to staging")).toBeTruthy();
    expect(lane(S.board.lanes.review).getAttribute("aria-label")).toBe(S.board.of(S.board.lanes.review, 2));
  });

  it("badges what needs you and can filter to it", async () => {
    mount(ok);
    await screen.findByText("Token bucket");
    expect(screen.getAllByText(S.board.needsYou)).toHaveLength(2); // the question and the plan
    fireEvent.click(screen.getByRole("button", { name: S.board.needsYouFilter }));
    expect(screen.queryByText("Token bucket")).toBeNull();
    expect(screen.getByText("Retry-After")).toBeTruthy();
    expect(screen.getByText("Deploy to staging")).toBeTruthy();
  });

  it("is read-only: no card can be dragged, and it says so", async () => {
    mount(ok);
    await screen.findByText("Token bucket");
    expect(document.querySelector("[draggable=true]")).toBeNull();
    expect(screen.getByText(S.board.readOnly)).toBeTruthy();
  });

  it("shows the criteria count on a card", async () => {
    mount(ok);
    await screen.findByText("Dashboard");
    expect(screen.getByText(S.board.criteria(1, 2))).toBeTruthy();
  });

  it("says when the project has no campaign yet", () => {
    mount(ok, { ...current, focus: null, runId: null });
    expect(screen.getByText(S.board.noCampaign)).toBeTruthy();
  });

  it("says the project list could not be read, not that there is no campaign, and retries the list", async () => {
    const ipc = fakeIpc({});
    setTransport(ipc.transport);
    render(<ProjectsProvider><CurrentProject.Provider value={{ project: null, campaigns: [], focus: null, runId: null }}><Board /></CurrentProject.Provider></ProjectsProvider>);
    expect(await screen.findByText(S.frame.loadFailed)).toBeTruthy();
    expect(screen.queryByText(S.board.noCampaign)).toBeNull();
    const before = ipc.calls.filter((c) => c.cmd === "list_campaigns").length;
    fireEvent.click(within(screen.getByRole("alert")).getByRole("button", { name: S.states.retry }));
    await waitFor(() => expect(ipc.calls.filter((c) => c.cmd === "list_campaigns").length).toBeGreaterThan(before));
  });

  it("says when the board cannot be read, with a retry", async () => {
    mount({});
    expect(await screen.findByText(S.board.failed)).toBeTruthy();
    expect(screen.getByRole("button", { name: S.states.retry })).toBeTruthy();
  });

  it("still shows the board when only the inbox fails", async () => {
    const { inbox: _, ...rest } = ok;
    mount(rest);
    await waitFor(() => expect(screen.getByText("Token bucket")).toBeTruthy());
    expect(screen.queryByText("Deploy to staging")).toBeNull();
  });

  it("Given two loads of the same campaign, When the older one answers last, Then the newer board stays", async () => {
    let calls = 0;
    let answerOld: (v: unknown) => void = () => {};
    const renamed = { ...overview, graph: { ...overview.graph, tasks: [task("t2", "in_progress", "Token bucket, newer")] } };
    const f = fakeIpc({ ...ok, run_overview: () => (++calls === 1 ? new Promise((r) => { answerOld = r; }) : renamed) });
    setTransport(f.transport);
    render(<CurrentProject.Provider value={current}><Board /></CurrentProject.Provider>);
    await waitFor(() => expect(calls).toBe(1));
    act(() => { f.emit("changed", {}); }); // a second load for the same campaign, while the first is still out
    expect(await screen.findByText("Token bucket, newer", undefined, { timeout: 1500 })).toBeTruthy();
    await act(async () => { answerOld(overview); });
    expect(screen.getByText("Token bucket, newer")).toBeTruthy();
    expect(screen.queryByText("Design the limiter")).toBeNull();
  });
});
