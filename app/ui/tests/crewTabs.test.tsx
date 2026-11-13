import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { setTransport } from "../src/ipc";
import { Crew } from "../src/screens/crew/Crew";
import { S } from "../strings";
import { fakeIpc } from "./fakeIpc";
import { Live, liveRun } from "./liveRun";

afterEach(() => {
  cleanup();
  setTransport();
  window.location.hash = "";
  vi.restoreAllMocks();
});

// The shape crew/tree.rs builds: a worker is a "round N" under the task group `task:<tid>`, which sits under the run.
const node = (id: string, kind: string, title: string, status: string, children: unknown[] = [], extra: object = {}) =>
  ({ id, kind, title, engine_id: "bhima", role: "Backend", status, overlays: [], children, ...extra });
const tree = node("lead", "lead", "Lead", "in_progress", [
  node("run:r1", "run", "Run r1", "in_progress", [
    node("task:t2", "task", "Backend · t2", "in_progress", [node("agent:a1", "round", "round 1", "in_progress")]),
    node("verification:r1", "verification", "Verification r1", "unknown", [node("agent:v1", "worker", "Tester", "unknown", [], { role: "Tester", engine_id: "vidura" })]),
  ]),
]);
const info = { worker: "a1", worktree: "/repo/.claude/worktrees/w1", branch: "worker-1", base: "abc123" };
const DIFF = "diff --git a/f.txt b/f.txt\n--- a/f.txt\n+++ b/f.txt\n@@ -1 +1,2 @@\n one\n+two\n";
const task = {
  task: { id: "r1.t2", title: "Add the thing", owner_agent: "backend", deps: ["r1.t1"], files: ["src/a.ts"], acceptance: ["it works"], attempts: 2, report: { status: "done", summary: "all green" } },
  stale: false, evidence: null,
};
const base = { active_runs: { count: 0, runs: [] }, crew: tree, tail: { lines: [], gap: false, head_seq: 0, spill_start: null, spill_end: null }, worktree_info: info };

async function open(replies: Record<string, unknown> = {}) {
  window.location.hash = "#/crew?slug=camp&id=r1";
  // A live run: its worker's task is polled.
  const f = fakeIpc({ ...base, ...liveRun(), ...replies });
  setTransport(f.transport);
  render(<Live><Crew /></Live>);
  fireEvent.click((await screen.findByText("round 1")).closest("[role=treeitem]") as HTMLElement);
  return f;
}
const tab = (name: string) => screen.findByRole("tab", { name });

describe("Worker tabs (S2c, US-2.3)", () => {
  it("offers Stream, Diff, Task and Worktree, Stream first", async () => {
    await open();
    const names = (await screen.findAllByRole("tab")).map((t) => t.textContent);
    expect(names).toEqual([S.crew.tabs.stream, S.crew.tabs.diff, S.crew.tabs.task, S.crew.tabs.worktree]);
  });

  it("Diff shows the worktree's git diff against its base, read only", async () => {
    const f = await open({ diff: { code: 0, diff: DIFF, truncated: false, timed_out: false } });
    fireEvent.click(await tab(S.crew.tabs.diff));
    await waitFor(() => expect(screen.getAllByText(/two/).length).toBeGreaterThan(0));
    expect(f.calls.find((c) => c.cmd === "diff")?.args).toEqual({ cwd: info.worktree, base: info.base });
    expect(screen.queryByRole("textbox")).toBeNull();
  });

  it("finds the worktree once the worker's transcript names it (first lookup fails, a later one succeeds)", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    let n = 0;
    const f = await open({ worktree_info: () => { if (n++ === 0) throw { error: "not_a_crew_worktree" }; return info; } });
    fireEvent.click(await tab(S.crew.tabs.worktree));
    expect(await screen.findByText(S.crew.noWorktree)).toBeTruthy();
    await act(async () => { await vi.advanceTimersByTimeAsync(3100); });
    expect((await screen.findAllByText(info.worktree, { exact: false })).length).toBeGreaterThan(0);
    expect(screen.queryByText(S.crew.noWorktree)).toBeNull();
    expect(f.calls.filter((c) => c.cmd === "worktree_info").length).toBeGreaterThanOrEqual(2);
    vi.useRealTimers();
  });

  it("keeps the Task tab (and the header title) current while the same worker and tab stay selected", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    let later = false;
    await open({
      task: () => {
        const t = { ...task.task, attempts: later ? 3 : 2, report: later ? { status: "done", summary: "second attempt green" } : task.task.report };
        return { ...task, task: t };
      },
    });
    fireEvent.click(await tab(S.crew.tabs.task));
    expect(await screen.findByText(/all green/)).toBeTruthy();
    later = true; // tasks.json changes while the same worker and tab stay selected
    await act(async () => { await vi.advanceTimersByTimeAsync(2100); });
    expect(await screen.findByText(/second attempt green/)).toBeTruthy();
    expect(screen.queryByText(/all green/)).toBeNull();
    vi.useRealTimers();
  });

  it("Diff says when there is nothing to show, was cut off, or failed", async () => {
    await open({ diff: { code: 0, diff: "", truncated: false, timed_out: false } });
    fireEvent.click(await tab(S.crew.tabs.diff));
    expect(await screen.findByText(S.crew.noChanges)).toBeTruthy();
  });

  it("Diff lists new untracked files instead of saying there are no changes", async () => {
    await open({ diff: { code: 0, diff: "", truncated: false, timed_out: false, untracked: ["new file.rs", "sub/also.txt"], untracked_truncated: true } });
    fireEvent.click(await tab(S.crew.tabs.diff));
    expect(await screen.findByText("new file.rs")).toBeTruthy();
    expect(screen.getByText("sub/also.txt")).toBeTruthy();
    expect(screen.getByText(S.crew.untrackedMore)).toBeTruthy();
    expect(screen.queryByText(S.crew.noChanges)).toBeNull();
  });

  it("Diff reports a truncated diff", async () => {
    await open({ diff: { code: 0, diff: DIFF, truncated: true, timed_out: false } });
    fireEvent.click(await tab(S.crew.tabs.diff));
    expect(await screen.findByText(S.crew.diffTruncated)).toBeTruthy();
  });

  it("Diff reports a git failure as an alert", async () => {
    await open({ diff: { code: 128, diff: "", truncated: false, timed_out: false } });
    fireEvent.click(await tab(S.crew.tabs.diff));
    expect(await screen.findByRole("alert")).toBeTruthy();
  });

  it("Task shows owner, deps, files, acceptance, attempts and the last report from the task graph id", async () => {
    const f = await open({ task });
    fireEvent.click(await tab(S.crew.tabs.task));
    const facts = await screen.findByRole("group", { name: S.crew.taskFacts });
    for (const t of ["backend", "r1.t1", "src/a.ts", "it works", "2", "all green"]) expect(facts.textContent).toContain(t);
    expect(f.calls.find((c) => c.cmd === "task")?.args).toEqual({ slug: "camp", taskId: "r1.t2" });
  });

  it("Task says so when the worker is not under a task group", async () => {
    window.location.hash = "#/crew?slug=camp&id=r1";
    const f = fakeIpc({ ...base });
    setTransport(f.transport);
    render(<Crew />);
    fireEvent.click((await screen.findByText("Tester")).closest("[role=treeitem]") as HTMLElement);
    fireEvent.click(await tab(S.crew.tabs.task));
    const panel = await screen.findByRole("tabpanel");
    expect(panel.textContent).toContain(S.crew.noTask);
    expect(f.calls.some((c) => c.cmd === "task")).toBe(false);
  });

  it("finds the task from the tree, not from the node title (a round is titled 'round 1')", async () => {
    const f = await open({ task });
    fireEvent.click(await tab(S.crew.tabs.task));
    await screen.findByRole("group", { name: S.crew.taskFacts });
    expect(f.calls.filter((c) => c.cmd === "task").every((c) => c.args?.taskId === "r1.t2")).toBe(true);
  });

  it("shows the role name from the core and the engine id as a secondary field", async () => {
    await open();
    const head = await screen.findByRole("group", { name: S.crew.header });
    expect(within(head).getByText("Backend")).toBeTruthy();
    expect(head.textContent).toContain(S.crew.engineId);
    expect(head.textContent).toContain("bhima");
  });

  it("Diff reports a timed-out diff as a timeout, not as a git exit code", async () => {
    await open({ diff: { code: null, diff: "", truncated: false, timed_out: true } });
    fireEvent.click(await tab(S.crew.tabs.diff));
    expect(await screen.findByText(S.crew.diffTimedOut)).toBeTruthy();
    expect(screen.queryByText(/git exit code/)).toBeNull();
  });

  it("Worktree shows path, branch and base, each copyable to the clipboard only", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
    const f = await open();
    fireEvent.click(await tab(S.crew.tabs.worktree));
    const facts = await screen.findByRole("group", { name: S.crew.worktreeFacts });
    for (const t of [info.worktree, info.branch, info.base]) expect(facts.textContent).toContain(t);
    fireEvent.click(within(facts).getByRole("button", { name: S.crew.copy(S.crew.worktreePath) }));
    expect(writeText).toHaveBeenCalledWith(info.worktree);
    expect(f.calls.map((c) => c.cmd)).not.toContain("reveal");
  });

  it("Worktree says when the worker has no crew worktree", async () => {
    await open({ worktree_info: new Error("x") });
    fireEvent.click(await tab(S.crew.tabs.worktree));
    expect(await screen.findByText(S.crew.noWorktree)).toBeTruthy();
  });

  it("the header names the worker's task and copyable worktree and branch", async () => {
    await open({ task });
    const head = await screen.findByRole("group", { name: S.crew.header });
    await waitFor(() => expect(head.textContent).toContain("worker-1"));
    expect(head.textContent).toContain("Add the thing");
    expect(head.textContent).toContain(info.worktree);
  });

  it("arrow keys move between tabs", async () => {
    await open();
    const first = await tab(S.crew.tabs.stream);
    first.focus();
    fireEvent.keyDown(first, { key: "ArrowRight" });
    expect(document.activeElement?.textContent).toBe(S.crew.tabs.diff);
  });
});
