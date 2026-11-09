// Closing finished workers (owner request 2026-11-09): the sidebar shows closed and unsaved workers, "Close finished"
// closes what is safe and lists what it kept, and "Close anyway" names exactly what would be lost before it asks the core.
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { setTransport } from "../src/ipc";
import type { CrewNodeDto } from "../src/ipc";
import { ProjectColumn } from "../src/shell/ProjectColumn";
import { WorkerPane } from "../src/screens/crew/WorkerPane";
import { CurrentProject } from "../src/projects/current";
import type { Project } from "../src/projects/contract";
import { S } from "../strings";
import { fakeIpc } from "./fakeIpc";

// jsdom has no modal dialog: model showModal/close the way browsers do.
beforeAll(() => {
  HTMLDialogElement.prototype.showModal = function (this: HTMLDialogElement) { this.setAttribute("open", ""); };
  HTMLDialogElement.prototype.close = function (this: HTMLDialogElement) { this.removeAttribute("open"); };
});
afterEach(() => { cleanup(); setTransport(); });

const project = { schema: "vajra.project/1", id: "p_1", name: "api", root: "/src/api", created: null, archived: null, rev: 1, registered: true } as unknown as Project;
const crew = { campaign: "c1", runId: "r1", workers: [
  { id: "w-open", name: "Backend", status: "done", statusLabel: "Done", task: "t1" },
  { id: "w-closed", name: "Reviewer", status: "done", statusLabel: "Done", task: "t2" },
  { id: "w-unsaved", name: "Frontend", status: "done", statusLabel: "Done", task: "t3" },
] } as never;
const rows = { workers: [
  { worker: "w-open", state: "closable" },
  { worker: "w-closed", state: "closed" },
  { worker: "w-unsaved", state: "unsaved_work", changed_files: 3, unpushed_commits: 2, no_upstream: false },
] };
const column = () => render(<ProjectColumn project={project} campaigns={[]} crew={crew} route={{ screen: "orch", params: {}, project: "p_1" }} onOrch={() => {}} onWorker={() => {}} onSession={() => {}} />);

describe("closing finished workers: sidebar", () => {
  it("marks closed workers and workers with unsaved work, with the exact counts", async () => {
    setTransport(fakeIpc({ closeout_status: rows }).transport);
    column();
    expect(await screen.findByText(S.closeout.closed)).toBeTruthy();
    const warn = screen.getByText(S.closeout.unsaved);
    expect(warn.getAttribute("title")).toBe("3 changed files, 2 unpushed commits");
  });

  it("Close finished sends one guarded action and lists what it kept and why", async () => {
    const f = fakeIpc({
      closeout_status: rows,
      act_close_finished: { receipt: { state: "done" }, result: { closed: [{ worker: "w-open", label: "t1" }], skipped: [{ worker: "w-unsaved", label: "t3", reason: "unsaved_work", changed_files: 3, unpushed_commits: 2, no_upstream: true }] } },
    });
    setTransport(f.transport);
    column();
    await screen.findByText(S.closeout.closed);
    fireEvent.click(screen.getByRole("button", { name: S.closeout.closeFinished }));
    const status = await screen.findByText(S.closeout.closedCount(1));
    expect(status).toBeTruthy();
    expect(screen.getByText("Frontend: unsaved work (3 changed files, 2 unpushed commits, no upstream branch)")).toBeTruthy();
    const call = f.calls.find((c) => c.cmd === "act_close_finished");
    expect((call?.args?.envelope as { project_id: string }).project_id).toBe("p_1");
  });

  it("is disabled when no worker is finished and open", async () => {
    setTransport(fakeIpc({ closeout_status: { workers: [{ worker: "w-open", state: "working" }, { worker: "w-closed", state: "closed" }] } }).transport);
    column();
    await screen.findByText(S.closeout.closed);
    expect(screen.getByRole("button", { name: S.closeout.closeFinished }).getAttribute("aria-disabled")).toBe("true");
  });

  it("shows nothing invented when the core cannot answer", async () => {
    setTransport(fakeIpc({}).transport);
    column();
    await waitFor(() => expect(screen.getByText("Backend")).toBeTruthy());
    expect(screen.queryByText(S.closeout.closed)).toBeNull();
    expect(screen.queryByText(S.closeout.unsaved)).toBeNull();
  });
});

describe("closing finished workers: Close anyway", () => {
  const node = { id: "agent:w-unsaved", kind: "worker", title: "Frontend", role: "Frontend", status: "done", engine_id: null, overlays: [], children: [] } as unknown as CrewNodeDto;
  const pane = (f: ReturnType<typeof fakeIpc>) => {
    setTransport(f.transport);
    render(<CurrentProject.Provider value={{ project, campaigns: [], focus: "c1", runId: "r1" }}><WorkerPane node={node} slug="c1" runId="r1" taskLocalId={null} status="done" /></CurrentProject.Provider>);
  };

  it("says what is unsaved, asks for confirmation naming it, and sends the counts that were confirmed", async () => {
    const f = fakeIpc({ closeout_status: rows, act_close_worker: { receipt: { state: "done" }, result: { closed: true } } });
    pane(f);
    const group = await screen.findByRole("group", { name: S.closeout.unsaved });
    expect(within(group).getByText(S.closeout.hasUnsaved("3 changed files, 2 unpushed commits"))).toBeTruthy();
    fireEvent.click(within(group).getByRole("button", { name: S.closeout.closeAnyway }));
    const dialog = await screen.findByRole("alertdialog");
    expect(within(dialog).getByText(S.closeout.confirmBody("3 changed files, 2 unpushed commits"))).toBeTruthy();
    expect(f.calls.some((c) => c.cmd === "act_close_worker")).toBe(false);
    fireEvent.click(within(dialog).getByRole("button", { name: S.closeout.confirm }));
    await waitFor(() => expect(f.calls.some((c) => c.cmd === "act_close_worker")).toBe(true));
    const args = f.calls.find((c) => c.cmd === "act_close_worker")?.args;
    expect([args?.worker, args?.changedFiles, args?.unpushedCommits]).toEqual(["w-unsaved", 3, 2]);
  });

  it("cancelling sends nothing, and a stale count is shown fresh to confirm again", async () => {
    const f = fakeIpc({ closeout_status: rows, act_close_worker: { receipt: { state: "failed" }, error: { error: "unsaved_work_changed", changed_files: 5, unpushed_commits: 2, no_upstream: false } } });
    pane(f);
    fireEvent.click(await screen.findByRole("button", { name: S.closeout.closeAnyway }));
    fireEvent.click(within(await screen.findByRole("alertdialog")).getByRole("button", { name: S.cancel }));
    expect(f.calls.some((c) => c.cmd === "act_close_worker")).toBe(false);
    fireEvent.click(screen.getByRole("button", { name: S.closeout.closeAnyway }));
    fireEvent.click(within(await screen.findByRole("alertdialog")).getByRole("button", { name: S.closeout.confirm }));
    expect(await screen.findByText(S.closeout.changed("5 changed files, 2 unpushed commits"))).toBeTruthy();
  });
});
