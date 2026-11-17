import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { setTransport } from "../src/ipc";
import type { CrewNodeDto } from "../src/ipc";
import { Crew } from "../src/screens/crew/Crew";
import { LiveFeed, ANNOUNCE_MS, FEED_ROWS } from "../src/screens/crew/LiveFeed";
import { STALL_MS, WATCH_MS, activeWorker, announceOf, feedItems, initialPick, stalledSince } from "../src/screens/crew/crewLive";
import { S } from "../strings";
import { fakeIpc } from "./fakeIpc";

afterEach(() => {
  cleanup();
  setTransport();
  window.location.hash = "";
  vi.useRealTimers();
});

const node = (id: string, kind: string, title: string, status: string, children: CrewNodeDto[] = [], extra: Partial<CrewNodeDto> = {}): CrewNodeDto =>
  ({ id, kind, title, engine_id: null, status, overlays: [], children, ...extra });
const lines = (texts: string[], from = 1) => texts.map((text, i) => ({ seq: from + i, text }));
const window_ = (texts: string[], from = 1) => ({ lines: lines(texts, from), head_seq: from + texts.length - 1, first_seq: 1, at_start: from === 1, at_end: true, gap: false });

/** The owner's run: 6 Nov, no process, tasks.json still says t1 is in progress, nothing picked it up. */
const stalledTree = node("lead", "lead", "Lead", "unknown", [
  node("run:r1", "run", "Run r1", "unknown", [node("task:t1", "task", "Prototyper · t1", "in_progress")]),
]);
const liveTree = node("lead", "lead", "Lead", "in_progress", [
  node("run:r1", "run", "Run r1", "in_progress", [
    node("task:t1", "task", "Design · t1", "done", [node("agent:a0", "round", "round 1", "done", [], { role: "Architect" })]),
    node("task:t2", "task", "Backend · t2", "in_progress", [
      node("agent:a1", "round", "round 1", "failed", [], { role: "Backend" }),
      node("agent:a2", "round", "round 2", "in_progress", [], { role: "Backend" }),
    ]),
    node("task:t3", "task", "Docs · t3", "pending"),
  ]),
]);

describe("crew live rules", () => {
  it("picks the working worker over the tree root, the newest round on a tie, and one reporting activity first", () => {
    expect(initialPick(liveTree)).toBe("agent:a2");
    expect(activeWorker(liveTree)?.id).toBe("agent:a2");
    const t = structuredClone(liveTree);
    t.children[0].children.push(node("core:r1.t9.r1", "round", "round 1", "in_progress"));
    expect(activeWorker(t)?.id).toBe("agent:a2"); // a worker with a stream to watch beats one with none
    t.children[0].children[2].children = [node("agent:b", "round", "round 1", "in_progress", [], { overlays: ["activity"] })];
    expect(activeWorker(t)?.id).toBe("agent:b");
  });

  it("opens on the run when no worker exists yet, never the Lead", () => {
    expect(initialPick(stalledTree)).toBe("run:r1");
  });

  it("calls a run stalled when no process runs it, or when it has been silent too long while watched", () => {
    const now = Date.UTC(2026, 10, 17);
    const base = { working: true, live: false, lastMs: now - 1000, watchedMs: 0, activeNow: false, now };
    expect(stalledSince(base)).toBe(now - 1000);
    expect(stalledSince({ ...base, lastMs: null })).toBeNull();
    expect(stalledSince({ ...base, working: false })).toBeUndefined();
    const live = { ...base, live: true, lastMs: now - STALL_MS - 1 };
    expect(stalledSince({ ...live, watchedMs: WATCH_MS - 1 })).toBeUndefined(); // not watched long enough to know
    expect(stalledSince({ ...live, watchedMs: WATCH_MS })).toBe(now - STALL_MS - 1);
    expect(stalledSince({ ...live, watchedMs: WATCH_MS, activeNow: true })).toBeUndefined();
    expect(stalledSince({ ...live, watchedMs: WATCH_MS, lastMs: now - 60_000 })).toBeUndefined();
  });

  it("reads the worker stream as messages, steps with their results, and prompts, with colour codes dropped", () => {
    const items = feedItems(lines([
      "[user] Build the bucket.", "[user] Use Redis.",
      "Reading first.", "Then editing.",
      "[tool] Bash npm test", "[result] \u001b[32mok\u001b[0m 3 passed",
      "[hook] Message from Lead: hurry",
      "[result] stray",
    ]));
    expect(items.map((i) => i.kind)).toEqual(["prompt", "say", "step", "note", "note"]);
    expect(items[0]).toMatchObject({ text: "Build the bucket.\nUse Redis." });
    expect(items[1]).toMatchObject({ text: "Reading first.\nThen editing." });
    expect(items[2]).toMatchObject({ tool: "Bash", detail: "npm test", result: ["ok 3 passed"] });
    expect(announceOf(items[2])).toBe("Bash npm test");
  });
});

describe("the Workers screen says what is happening", () => {
  const open = (replies: Record<string, unknown>) => {
    const f = fakeIpc({ active_runs: { count: 0, runs: [] }, task: { task: {}, stale: false }, worktree_info: { worker: "a2", worktree: "/w", branch: "b", base: null }, ...replies });
    setTransport(f.transport);
    window.location.hash = "#/crew?slug=camp&id=r1";
    render(<Crew />);
    return f;
  };

  it("a stalled run reads Stalled, never a blue In progress, and offers Review", async () => {
    open({ crew: stalledTree });
    const t = await screen.findByRole("tree");
    const row = within(t).getByText("Prototyper · t1").closest("[role=treeitem]") as HTMLElement;
    expect(row.textContent).toContain(S.crew.stalled);
    expect(row.textContent).not.toContain(S.status.in_progress);
    // It opens on the run, with the stall said in words and the one thing to do.
    const note = await screen.findByRole("group", { name: S.crew.stalled });
    expect(note.textContent).toMatch(/^Stalled: no activity/);
    expect(within(note).getByRole("button", { name: S.crew.review })).toBeTruthy();
    fireEvent.click(row);
    const pane = await screen.findByRole("region", { name: "Prototyper · t1" });
    expect(pane.textContent).toContain(S.crew.neverStarted);
    expect(screen.queryByText(S.crew.kind)).toBeNull();
  });

  it("names when a stalled run last moved, from the campaign's own record", async () => {
    const f = fakeIpc({ active_runs: { count: 0, runs: [] }, crew: stalledTree, list_campaigns: { campaigns: [{ slug: "camp", updated: "2026-11-06T22:05:00Z" }] }, resume_targets: { targets: {} }, inbox: { items: [], count: 0 }, project_list: { projects: [] } });
    setTransport(f.transport);
    window.location.hash = "#/crew?slug=camp&id=r1";
    const { Live } = await import("./liveRun");
    render(<Live><Crew /></Live>);
    const note = await screen.findByRole("group", { name: S.crew.stalled });
    await waitFor(() => expect(note.textContent).toContain("since"));
    expect(note.textContent).toMatch(/Nov|11/);
  });

  it("a node nothing reports on says 'No live updates' and why, never 'Unknown (no signal)'", async () => {
    open({ crew: node("lead", "lead", "Lead", "unknown", [node("run:r1", "run", "Run r1", "unknown", [node("task:t1", "task", "Docs · t1", "pending")])]) });
    const t = await screen.findByRole("tree");
    const run = within(t).getByText("Run r1").closest("[role=treeitem]") as HTMLElement;
    expect(run.textContent).toContain(S.crew.unknownSignal);
    expect(run.textContent).not.toMatch(/no signal/i);
    expect(run.querySelector(".v-row__meta")?.getAttribute("title")).toContain(S.crew.noLiveWhy.run);
  });

  it("a task with no worker in a running run says it is waiting to start", async () => {
    open({ crew: liveTree, stream_window: window_([]) });
    fireEvent.click(within(await screen.findByRole("tree")).getByText("Docs · t3").closest("[role=treeitem]") as HTMLElement);
    expect((await screen.findByRole("region", { name: "Docs · t3" })).textContent).toContain(S.crew.waiting);
  });

  it("opens on the active worker's Live tab, and a task opens its latest worker", async () => {
    const f = open({ crew: liveTree, stream_window: window_(["Working on it."]) });
    const t = await screen.findByRole("tree");
    await waitFor(() => expect((within(t).getByText("round 2").closest("[role=treeitem]") as HTMLElement).getAttribute("aria-selected")).toBe("true"));
    expect((await screen.findByRole("tab", { name: S.crew.tabs.live })).getAttribute("aria-selected")).toBe("true");
    await waitFor(() => expect(screen.getByRole("log", { name: S.crew.live.label }).textContent).toContain("Working on it."));
    expect(f.calls.find((c) => c.cmd === "stream_window")?.args).toMatchObject({ stream: "w-a2", maxLines: FEED_ROWS });
  });

  it("the run shows who is working on what, and that worker's activity under it", async () => {
    open({ crew: liveTree, stream_window: window_(["[tool] Edit src/a.ts"]) });
    fireEvent.click(within(await screen.findByRole("tree")).getByText("Run r1").closest("[role=treeitem]") as HTMLElement);
    const sum = await screen.findByRole("region", { name: S.crew.summary.label });
    expect(sum.textContent).toContain(S.crew.summary.working("Backend", "Backend · t2"));
    expect(sum.textContent).toContain(S.crew.workersDone(1, 3));
    const pane = screen.getByRole("region", { name: "Run r1" });
    expect(within(pane).getByRole("heading", { name: S.crew.summary.streamOf("Backend") })).toBeTruthy();
    await waitFor(() => expect(within(pane).getByRole("log", { name: S.crew.live.label }).textContent).toContain("src/a.ts"));
  });
});

describe("LiveFeed", () => {
  it("is not itself a live region; one polite status speaks, at most once per interval, and not the backlog", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    let head = 2;
    const all = () => Array.from({ length: head }, (_, i) => `[tool] Step${i + 1} x`);
    const f = fakeIpc({
      stream_window: () => window_(all()),
      tail: (a?: Record<string, unknown>) => ({ lines: lines(all().slice(a?.afterSeq as number), (a?.afterSeq as number) + 1), gap: false, head_seq: head, spill_start: null, spill_end: null }),
    });
    setTransport(f.transport);
    render(<LiveFeed stream="w-a" />);
    const log = await screen.findByRole("log", { name: S.crew.live.label });
    expect(log.getAttribute("aria-live")).toBe("off");
    const status = screen.getByRole("status");
    await waitFor(() => expect(log.textContent).toContain("Step2"));
    expect(status.textContent).toBe("");
    head = 3;
    await act(async () => { await vi.advanceTimersByTimeAsync(ANNOUNCE_MS + 1100); });
    await waitFor(() => expect(status.textContent).toBe(S.crew.live.announce("Step3 x")));
    head = 4;
    await act(async () => { await vi.advanceTimersByTimeAsync(1100); });
    await waitFor(() => expect(log.textContent).toContain("Step4"));
    expect(status.textContent).toBe(S.crew.live.announce("Step3 x")); // too soon to speak again
  });

  it("scrolling up pauses following: new lines are counted, not shown, until Follow live", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    let head = 2;
    const all = () => Array.from({ length: head }, (_, i) => `line ${i + 1}`);
    const f = fakeIpc({
      stream_window: () => window_(all()),
      tail: (a?: Record<string, unknown>) => ({ lines: lines(all().slice(a?.afterSeq as number), (a?.afterSeq as number) + 1), gap: false, head_seq: head, spill_start: null, spill_end: null }),
    });
    setTransport(f.transport);
    const seen: number[] = [];
    render(<LiveFeed stream="w-a" onActivity={(at) => seen.push(at)} />);
    const log = await screen.findByRole("log", { name: S.crew.live.label });
    await waitFor(() => expect(log.textContent).toContain("line 2"));
    // jsdom has no layout: make the box taller than its view and scroll it up.
    Object.defineProperty(log, "scrollHeight", { configurable: true, value: 1000 });
    Object.defineProperty(log, "clientHeight", { configurable: true, value: 200 });
    act(() => { log.scrollTop = 100; fireEvent.scroll(log); });
    expect(await screen.findByText(S.crew.live.paused)).toBeTruthy();
    head = 5;
    await act(async () => { await vi.advanceTimersByTimeAsync(1100); });
    await waitFor(() => expect(screen.getByText(new RegExp(S.crew.live.newLines(3)))).toBeTruthy());
    expect(log.textContent).not.toContain("line 5");
    expect(seen.length).toBeGreaterThan(0); // growth was still noticed
    fireEvent.click(screen.getByRole("button", { name: S.crew.live.follow }));
    await waitFor(() => expect(log.textContent).toContain("line 5"));
    expect(screen.queryByText(S.crew.live.paused)).toBeNull();
  });

  it("holds at most FEED_ROWS rows however long the stream runs", async () => {
    const f = fakeIpc({ stream_window: (a?: Record<string, unknown>) => window_(Array.from({ length: a?.maxLines as number }, (_, i) => `say ${i}`), 9000) });
    setTransport(f.transport);
    render(<LiveFeed stream="w-a" live={false} />);
    await waitFor(() => expect(screen.getByRole("log").textContent).toContain(`say ${FEED_ROWS - 1}`));
    expect(f.calls[0].args).toMatchObject({ from: null, maxLines: FEED_ROWS });
  });
});
