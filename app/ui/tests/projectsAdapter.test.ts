// FG-01 / FG-03 / FG-06 (feature gap matrix) against the UI-side adapter over today's APIs. When the core's project
// contract (ADR-15, T1 PR 1a) lands, these become the parity tests for the swap.
import { describe, expect, it } from "vitest";
import { boardCards, groupProjects, projectId, sessionsByDay, summarize } from "../src/projects/adapter";
import type { Campaign, EvidenceRow, InboxItem, RunOverview } from "../src/ipc";

const camp = (slug: string, extra: Partial<Campaign> = {}): Campaign => ({ slug, goal: `Goal ${slug}`, status: "done", updated: "2026-11-06 10:00", ...extra });
const target = (cwd: string) => ({ session_id: "s", run_id: "r", cwd });
const inbox = (campaign: string, kind: InboxItem["kind"] = "question", title = "Which key?"): InboxItem =>
  ({ id: `i-${campaign}-${title}`, kind, campaign, origin: "app", created: "", age_s: 1, title, body: "", actions: [] });

describe("groupProjects (FG-01)", () => {
  it("Given runs in two folders, each folder is its own project with only its own campaigns", () => {
    const { projects, byCampaign } = groupProjects(
      [camp("a1"), camp("b1"), camp("a2")],
      { a1: target("/src/api"), a2: target("/src/api/"), b1: target("/src/web") },
    );
    expect(projects.map((p) => p.name)).toEqual(["api", "web"]);
    expect(projects[0].root).toBe("/src/api");
    expect(byCampaign.a1).toBe(projects[0].id);
    expect(byCampaign.a2).toBe(projects[0].id); // a trailing slash is the same folder
    expect(byCampaign.b1).toBe(projects[1].id);
    expect(projects.every((p) => p.schema === "vajra.project/1")).toBe(true);
  });

  it("never assigns a campaign without a recorded folder: it goes to the Unsorted bucket, listed last", () => {
    const { projects, byCampaign } = groupProjects([camp("x"), camp("a1")], { a1: target("/src/api") });
    expect(projects.map((p) => p.bucket ?? "folder")).toEqual(["folder", "unsorted"]);
    expect(projects[1].root).toBeNull();
    expect(byCampaign.x).toBe(projects[1].id);
  });

  it("has no Unsorted bucket when every campaign has a folder, and no projects with no campaigns", () => {
    expect(groupProjects([], {}).projects).toEqual([]);
    expect(groupProjects([camp("a1")], { a1: target("/src/api") }).projects).toHaveLength(1);
  });

  it("orders projects by their oldest campaign, so a new run never reshuffles the tabs (Cmd+1..n stay put)", () => {
    const { projects } = groupProjects(
      [camp("new-web", { created: "2026-11-08 09:00" }), camp("old-api", { created: "2026-11-01 09:00" }), camp("old-web", { created: "2026-10-30 09:00" })],
      { "new-web": target("/src/web"), "old-api": target("/src/api"), "old-web": target("/src/web") },
    );
    expect(projects.map((p) => p.name)).toEqual(["web", "api"]);
  });

  it("ids are stable for a folder and safe in a URL path segment", () => {
    expect(projectId("/src/api")).toBe(projectId("/src/api/"));
    expect(projectId("/src/api")).not.toBe(projectId("/src/web"));
    expect(projectId("/src/a b")).toMatch(/^p_[0-9a-f]+$/);
  });

  it("tolerates a core that answers with nothing (perf fixture, old core)", () => {
    expect(groupProjects(undefined as unknown as Campaign[], undefined as unknown as Record<string, never>).projects).toEqual([]);
  });
});

describe("summarize: the tab state (FG-03 'urgent items stay visible')", () => {
  const idx = groupProjects([camp("a1"), camp("b1", { status: "stopped", updated: "2026-11-07 10:00" }), camp("b0", { updated: "2026-11-01 10:00" })],
    { a1: target("/src/api"), b1: target("/src/web"), b0: target("/src/web") });
  const api = idx.projects.find((p) => p.name === "api")!;
  const web = idx.projects.find((p) => p.name === "web")!;

  it("counts needs-you items per project from the inbox, scoped by campaign", () => {
    const s = summarize(idx, [camp("a1"), camp("b1", { status: "stopped" }), camp("b0")], [inbox("a1"), inbox("a1", "plan", "Deploy"), inbox("b1")], null);
    expect(s.get(api.id)?.needsYou).toBe(2);
    expect(s.get(web.id)?.needsYou).toBe(1);
  });

  it("marks a project whose newest campaign stopped or failed; needs-you wins over it", () => {
    const camps = [camp("a1"), camp("b1", { status: "stopped", updated: "2026-11-07 10:00" }), camp("b0", { updated: "2026-11-01 10:00" })];
    expect(summarize(idx, camps, [], null).get(web.id)?.state).toBe("stopped");
    expect(summarize(idx, camps, [inbox("b1")], null).get(web.id)?.state).toBe("needs_you");
    expect(summarize(idx, camps, [], null).get(api.id)?.state).toBeNull();
  });

  it("gives worker pips only to the project that owns the active run", () => {
    const crew = { campaign: "a1", runId: "r1", workers: [{ id: "backend", name: "Backend", status: "in_progress" as const, statusLabel: "In progress", task: "t2" }] };
    const s = summarize(idx, [camp("a1"), camp("b1"), camp("b0")], [], crew);
    expect(s.get(api.id)?.pips).toEqual(["in_progress"]);
    expect(s.get(web.id)?.pips).toEqual([]);
  });
});

describe("sessionsByDay (FG-03 scoped history)", () => {
  const now = new Date("2026-11-08T12:00:00");
  it("groups a project's runs by day, newest first, with a status summary per day", () => {
    const days = sessionsByDay([
      camp("a", { updated: "2026-11-08 09:10", status: "done" }),
      camp("b", { updated: "2026-11-08 11:30", status: "stopped" }),
      camp("c", { updated: "2026-11-07 18:00", status: "done" }),
      camp("d", { updated: "2026-11-02 08:00", status: "failed" }),
    ], now);
    expect(days.map((d) => d.label)).toEqual(["Today", "Yesterday", "Mon 2 Nov"]);
    expect(days[0].items.map((s) => s.slug)).toEqual(["b", "a"]);
    expect(days[0].items[0].time).toBe("11:30");
    expect(days[0].summary).toBe("1 done, 1 stopped");
    expect(days[2].summary).toBe("1 failed");
  });

  it("puts runs with no readable time under one 'Earlier' group instead of dropping them", () => {
    const days = sessionsByDay([camp("a", { updated: undefined, created: undefined }), camp("b", { updated: "garbage" })], now);
    expect(days).toHaveLength(1);
    expect(days[0].label).toBe("Earlier");
    expect(days[0].items).toHaveLength(2);
  });
});

describe("boardCards (FG-06, ADR-15 §4.2: 4 lanes and a needs-you badge, read-only)", () => {
  const task = (local: string, status: string, title = `Task ${local}`) => ({ id: `r1.${local}`, local, title, owner: "bhima", status, deps: [], wave: 1 });
  const overview = (tasks: ReturnType<typeof task>[]): RunOverview => ({
    slug: "c", run_id: "r1", stale: false,
    timeline: { phases: [], current: "execute", status: "active" },
    graph: { tasks, waves: 1 },
  });
  const ev = (t: string, status: EvidenceRow["status"], criterion = "AC-1"): EvidenceRow =>
    ({ task: t, title: null, owner: null, criterion, status, head: null, path: null, commands: null, report: null });
  const lane = (cards: ReturnType<typeof boardCards>, local: string) => cards.find((c) => c.local === local)?.lane;

  it("puts planned work in Backlog and live attempts in Running", () => {
    const cards = boardCards(overview([task("t1", "pending"), task("t2", "ready"), task("t3", "claimed"), task("t4", "in_progress")]), [], []);
    expect(["t1", "t2", "t3", "t4"].map((t) => lane(cards, t))).toEqual(["backlog", "backlog", "running", "running"]);
  });

  it("moves a task to Done only when every criterion is verified passing; otherwise finished work waits in Review", () => {
    const cards = boardCards(overview([task("t1", "done"), task("t2", "done"), task("t3", "done"), task("t4", "failed")]),
      [ev("r1.t1", "pass"), ev("r1.t1", "pass", "AC-2"), ev("t2", "pass"), ev("t2", "fail", "AC-2")], []);
    expect(lane(cards, "t1")).toBe("done");
    expect(lane(cards, "t2")).toBe("review"); // one criterion fails
    expect(lane(cards, "t3")).toBe("review"); // "done" by the worker, but nothing verified
    expect(lane(cards, "t4")).toBe("review");
  });

  it("badges questions and blocked tasks as needing you, in the lane their work is in", () => {
    const cards = boardCards(overview([task("t1", "question"), task("t2", "blocked"), task("t3", "in_progress")]), [], []);
    expect(cards.find((c) => c.local === "t1")).toMatchObject({ lane: "running", needsYou: true });
    expect(cards.find((c) => c.local === "t2")).toMatchObject({ lane: "backlog", needsYou: true });
    expect(cards.find((c) => c.local === "t3")?.needsYou).toBe(false);
  });

  it("shows a plan awaiting approval as a Review card that needs you; other campaigns' items are ignored", () => {
    const cards = boardCards(overview([]), [], [inbox("c", "plan", "Deploy to staging"), inbox("other", "plan", "Not mine")]);
    expect(cards).toHaveLength(1);
    expect(cards[0]).toMatchObject({ lane: "review", needsYou: true, kind: "plan", title: "Deploy to staging" });
  });

  it("counts criteria per card for the evidence line", () => {
    const [c] = boardCards(overview([task("t1", "done")]), [ev("r1.t1", "pass"), ev("r1.t1", "fail", "AC-2"), ev("r1.t1", "not_run", "AC-3")], []);
    expect(c.criteria).toEqual({ pass: 1, total: 3 });
  });
});
