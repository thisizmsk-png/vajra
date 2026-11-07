import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { setTransport } from "../src/ipc";
import { Run } from "../src/screens/run/Run";
import { S } from "../strings";
import { fakeIpc } from "./fakeIpc";

afterEach(() => {
  cleanup();
  setTransport();
  window.location.hash = "";
});

const row = (task: string, criterion: string, status: string, extra: object = {}) =>
  ({ task, title: `Task ${task}`, owner: "bhima", criterion, status, head: null, path: null, commands: null, report: null, ...extra });
const list = (rows: unknown[]) => ({ slug: "camp", stale: false, rows });
const open = async (rows: unknown[], extra: Record<string, unknown> = {}) => {
  window.location.hash = "#/run?slug=camp";
  const f = fakeIpc({ evidence_list: list(rows), ...extra });
  setTransport(f.transport);
  render(<Run />);
  await waitFor(() => expect(f.calls.some((c) => c.cmd === "evidence_list")).toBe(true));
  if (rows.length) await screen.findAllByRole("listitem");
  else await screen.findByText(S.run.none);
  return f;
};
const order = () => screen.getAllByRole("listitem").map((e) => e.textContent ?? "");

describe("Run view, Evidence (S4a, POS-1.1, US-4.3)", () => {
  it("drops a response for the previous campaign that resolves after the route changed", async () => {
    let release: (v: unknown) => void = () => {};
    const slowA = new Promise((r) => { release = r; });
    window.location.hash = "#/run?slug=camp-a";
    const f = fakeIpc({
      evidence_list: (a?: Record<string, unknown>) => (a?.slug === "camp-a" ? slowA : list([row("tb", "from-B", "pass")])),
    });
    setTransport(f.transport);
    render(<Run />);
    await waitFor(() => expect(f.calls.some((c) => c.args?.slug === "camp-a")).toBe(true));
    act(() => { window.location.hash = "#/run?slug=camp-b"; });
    await screen.findByText("from-B");
    await act(async () => { release({ slug: "camp-a", stale: false, rows: [row("ta", "from-A", "fail")] }); await Promise.resolve(); });
    expect(screen.queryByText("from-A")).toBeNull();
    expect(screen.getByText("from-B")).toBeTruthy();
  });

  it("shows unchecked criteria as Checking and asks again until the core has checked them all", async () => {
    let n = 0;
    window.location.hash = "#/run?slug=camp";
    const f = fakeIpc({
      evidence_list: () => (n++ === 0
        ? { slug: "camp", stale: false, pending: 1, rows: [row("t1", "ac-1", "checking")] }
        : { slug: "camp", stale: false, pending: 0, rows: [row("t1", "ac-1", "pass")] }),
    });
    setTransport(f.transport);
    render(<Run />);
    await screen.findByText(S.run.status.checking);
    await waitFor(() => expect(screen.getAllByRole("listitem")[0].textContent).toContain(S.run.status.pass), { timeout: 4000 });
    expect(f.calls.filter((c) => c.cmd === "evidence_list")).toHaveLength(2);
  });

  it("is the run's default view and asks the core for the campaign's criteria", async () => {
    const f = await open([row("t1", "ac-1", "pass")]);
    expect(f.calls.find((c) => c.cmd === "evidence_list")?.args).toEqual({ slug: "camp" });
    expect(screen.getByRole("heading", { name: S.run.evidence })).toBeTruthy();
  });

  it("lists failing criteria first, then unverified, not run, and passing last", async () => {
    await open([row("t1", "ac-pass", "pass"), row("t2", "ac-nr", "not_run"), row("t3", "ac-un", "unverified"), row("t4", "ac-fail", "fail")]);
    const o = order();
    expect(o[0]).toContain("ac-fail");
    expect(o[1]).toContain("ac-un");
    expect(o[2]).toContain("ac-nr");
    expect(o[3]).toContain("ac-pass");
  });

  it("states every status as text", async () => {
    await open([row("t1", "a", "pass"), row("t2", "b", "fail"), row("t3", "c", "not_run"), row("t4", "d", "unverified")]);
    const text = order().join(" ");
    for (const s of [S.run.status.pass, S.run.status.fail, S.run.status.not_run, S.run.status.unverified]) expect(text).toContain(s);
  });

  it("never shows a green summary while any criterion is not a pass", async () => {
    await open([row("t1", "a", "pass"), row("t2", "b", "unverified")]);
    expect(screen.queryByText(S.run.allPass)).toBeNull();
    expect(screen.getByText(S.run.summary(1, 2)).closest(".v-badge--success")).toBeNull();
  });

  it("shows the all-pass summary only when every criterion passes", async () => {
    await open([row("t1", "a", "pass"), row("t2", "b", "pass")]);
    expect(screen.getByText(S.run.allPass).closest(".v-badge--success")).toBeTruthy();
  });

  it("a campaign with no tasks has nothing to pass and no green badge", async () => {
    await open([]);
    expect(screen.queryByText(S.run.allPass)).toBeNull();
    expect(screen.getByText(S.run.none)).toBeTruthy();
  });

  const detail = (commands: unknown[], notes: string | null = null, truncated = false) => ({ evidence_detail: { task: "t2", commands, notes, truncated } });

  it("selecting a row shows its proof: commit, evidence file, each recorded command with its exit code, report", async () => {
    const f = await open(
      [row("t2", "ac-2", "unverified", { head: "a".repeat(40), path: "/camp/evidence/e.json", commands: 2, report: "tests red" })],
      detail([
        { cmd: "npm test", exit_code: 1, passed: false, output_tail: "FAIL src/a.test.ts" },
        { cmd: "tsc --noEmit", exit_code: 0, passed: true, output_tail: null },
      ], "verifier: two tests red"),
    );
    fireEvent.click(within(screen.getByRole("listitem")).getByRole("button"));
    const d = await screen.findByRole("group", { name: S.run.detail });
    await waitFor(() => expect(d.textContent).toContain("npm test"));
    for (const t of ["aaaaaaa", "/camp/evidence/e.json", "tests red", "t2", S.run.exit(1), S.run.exit(0), "tsc --noEmit", "FAIL src/a.test.ts", "verifier: two tests red"]) expect(d.textContent).toContain(t);
    expect(f.calls.find((c) => c.cmd === "evidence_detail")?.args).toEqual({ slug: "camp", taskId: "t2" });
  });

  it("shows who wrote the criterion's task and copies the literal command", async () => {
    const writeText = vi.fn();
    Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
    await open(
      [row("t2", "ac-2", "fail", { head: "a".repeat(40), path: "/camp/evidence/e.json", owner: "bhima" })],
      detail([{ cmd: "pytest -k large", exit_code: 1, passed: false, output_tail: null }]),
    );
    fireEvent.click(within(screen.getByRole("listitem")).getByRole("button"));
    const d = await screen.findByRole("group", { name: S.run.detail });
    expect(d.textContent).toContain("bhima");
    fireEvent.click(await within(d).findByRole("button", { name: S.crew.copy(S.run.copyWhat) }));
    expect(writeText).toHaveBeenCalledWith("pytest -k large");
  });

  it("keeps the detail on the criterion the user chose when a status change reorders the list", async () => {
    let n = 0;
    window.location.hash = "#/run?slug=camp";
    const first = [row("t1", "ac-a", "checking"), row("t2", "ac-b", "checking")];
    const second = [row("t1", "ac-a", "pass"), row("t2", "ac-b", "fail")]; // ac-b now sorts above ac-a
    const f = fakeIpc({
      evidence_list: () => ({ slug: "camp", stale: false, pending: n++ === 0 ? 2 : 0, rows: n === 1 ? first : second }),
      evidence_detail: { task: "t1", commands: [], notes: null, truncated: false },
    });
    setTransport(f.transport);
    render(<Run />);
    await screen.findAllByRole("listitem");
    fireEvent.click(within(screen.getAllByRole("listitem").find((e) => e.textContent?.includes("ac-a"))!).getByRole("button"));
    await waitFor(() => expect(screen.getByRole("group", { name: S.run.detail }).textContent).toContain("ac-a"));
    await waitFor(() => expect(screen.getAllByRole("listitem")[0].textContent).toContain("ac-b"), { timeout: 4000 });
    expect(screen.getByRole("group", { name: S.run.detail }).textContent).toContain("ac-a");
    expect(screen.getByRole("group", { name: S.run.detail }).textContent).not.toContain("ac-b");
  });

  it("shows command output as inert text, never as markup", async () => {
    await open(
      [row("t2", "ac-2", "fail", { head: "a".repeat(40), path: "/camp/evidence/e.json", commands: 1 })],
      detail([{ cmd: "<img src=x onerror=alert(1)>", exit_code: 2, passed: false, output_tail: "<script>window.__pwned=1</script>" }]),
    );
    fireEvent.click(within(screen.getByRole("listitem")).getByRole("button"));
    const d = await screen.findByRole("group", { name: S.run.detail });
    await waitFor(() => expect(d.textContent).toContain("<script>window.__pwned=1</script>"));
    expect(d.querySelector("script, img")).toBeNull();
    expect((window as unknown as { __pwned?: number }).__pwned).toBeUndefined();
  });

  it("says so when the commands cannot be read or the evidence file is too large", async () => {
    await open([row("t2", "ac-2", "fail", { head: "a".repeat(40), path: "/camp/evidence/e.json" })], { evidence_detail: new Error("boom") });
    fireEvent.click(within(screen.getByRole("listitem")).getByRole("button"));
    expect((await screen.findByRole("group", { name: S.run.detail })).textContent).toContain("");
    await waitFor(() => expect(screen.getByRole("group", { name: S.run.detail }).textContent).toContain(S.run.detailFailed));
    cleanup();
    await open([row("t2", "ac-2", "fail", { head: "a".repeat(40), path: "/camp/evidence/e.json" })], detail([], null, true));
    fireEvent.click(within(screen.getByRole("listitem")).getByRole("button"));
    await waitFor(() => expect(screen.getByRole("group", { name: S.run.detail }).textContent).toContain(S.run.detailTruncated));
  });

  it("a criterion without evidence says no evidence, never pass", async () => {
    await open([row("t1", "ac-1", "not_run")]);
    fireEvent.click(within(screen.getByRole("listitem")).getByRole("button"));
    expect((await screen.findByRole("group", { name: S.run.detail })).textContent).toContain(S.run.noEvidence);
  });

  it("renders only the rows in view for 2,000 criteria", async () => {
    await open(Array.from({ length: 2000 }, (_, i) => row(`t${i}`, `ac-${i}`, i === 0 ? "fail" : "pass")));
    expect(screen.getAllByRole("listitem").length).toBeLessThan(60);
    expect(screen.getByText(S.run.summary(1999, 2000))).toBeTruthy();
  });

  it("shows a read error as an alert and keeps the page", async () => {
    window.location.hash = "#/run?slug=camp";
    setTransport(fakeIpc({ evidence_list: new Error("x"), run_overview: overview() }).transport);
    render(<Run />);
    expect(await screen.findByRole("alert")).toBeTruthy();
  });

  it("asks for a campaign when none is chosen", async () => {
    window.location.hash = "#/run";
    const f = fakeIpc({});
    setTransport(f.transport);
    render(<Run />);
    expect(await screen.findByText(S.run.pick)).toBeTruthy();
    await waitFor(() => expect(f.calls).toHaveLength(0));
  });
});

const phases = (current: string, extra: Record<string, unknown> = {}) =>
  ["Understand", "Plan", "Execute", "Verify", "Fix", "Deliver"].map((name, i, all) => {
    const at = all.indexOf(current);
    return { name, state: i < at ? "done" : i === at ? "current" : "pending", ...(name === "Fix" ? extra : {}) };
  });
const gtask = (local: string, wave: number, status = "pending", deps: string[] = []) =>
  ({ id: `r1.${local}`, local, title: `Title ${local}`, owner: "Backend", status, deps, wave });
const overview = (over: Record<string, unknown> = {}) => ({
  slug: "camp", run_id: "r1", stale: false,
  timeline: { phases: phases("Execute"), current: "Execute", status: "active" },
  graph: { tasks: [gtask("t1", 0, "closed"), gtask("t2", 1, "in_progress", ["r1.t1"])], waves: 2 },
  ...over,
});

describe("Run view, timeline and task graph (S4c, US-4.1, US-4.2)", () => {
  const openOverview = async (o: unknown) => {
    window.location.hash = "#/run?slug=camp";
    const f = fakeIpc({ evidence_list: list([]), run_overview: o });
    setTransport(f.transport);
    render(<Run />);
    await screen.findByRole("list", { name: S.run.graph });
    return f;
  };

  it("marks exactly one phase current and states every phase as text (AC-4.1.1)", async () => {
    await openOverview(overview());
    const items = within(screen.getByRole("list", { name: S.run.timeline })).getAllByRole("listitem");
    expect(items).toHaveLength(6);
    expect(items.filter((i) => i.getAttribute("aria-current") === "step")).toHaveLength(1);
    expect(items[2].textContent).toContain(S.run.phaseState.current);
    expect(items[0].textContent).toContain(S.run.phaseState.done);
    expect(items[5].textContent).toContain(S.run.phaseState.pending);
  });

  it("shows the Fix round, with the limit only when it is known (AC-4.1.2)", async () => {
    await openOverview(overview({ timeline: { phases: phases("Fix", { round: 2, max_rounds: null }), current: "Fix", status: "active" } }));
    expect(screen.getByText(/round 2$/)).toBeTruthy();
    expect(screen.queryByText(/round 2 of/)).toBeNull();
    cleanup();
    await openOverview(overview({ timeline: { phases: phases("Fix", { round: 3, max_rounds: 3 }), current: "Fix", status: "need_input" } }));
    expect(screen.getByText(/round 3 of 3/)).toBeTruthy();
    expect(screen.getAllByText(S.run.runStatus.need_input)[0]).toBeTruthy();
  });

  it("shows each way a run ends as text", async () => {
    for (const status of ["done", "need_input", "rejected"] as const) {
      await openOverview(overview({ timeline: { phases: phases("Verify"), current: "Verify", status } }));
      expect(screen.getAllByText(S.run.runStatus[status])[0]).toBeTruthy();
      cleanup();
    }
  });

  it("lists tasks with wave, owner role, status text and what each waits for (AC-4.2.1)", async () => {
    await openOverview(overview());
    const rows = within(screen.getByRole("list", { name: S.run.graph })).getAllByRole("listitem");
    expect(rows[1].textContent).toContain(S.run.wave(1));
    expect(rows[1].textContent).toContain("Backend");
    expect(rows[1].textContent).toContain(S.run.taskStatus.in_progress);
    expect(rows[1].textContent).toContain(`${S.run.waitsFor} t1`);
  });

  it("renders only the rows in view for 200 tasks (AC-4.2.2)", async () => {
    const tasks = Array.from({ length: 200 }, (_, i) => gtask(`t${i}`, i, "pending", i ? [`r1.t${i - 1}`] : []));
    await openOverview(overview({ graph: { tasks, waves: 200 } }));
    expect(screen.getAllByRole("listitem").length).toBeLessThan(60);
  });

  it("Enter on a task opens that worker in the Crew view (AC-4.2.3)", async () => {
    await openOverview(overview());
    const row = within(screen.getByRole("list", { name: S.run.graph })).getAllByRole("listitem")[1];
    fireEvent.keyDown(row, { key: "Enter" });
    expect(window.location.hash).toContain("#/crew");
    const q = new URLSearchParams(window.location.hash.split("?")[1]);
    expect(Object.fromEntries(q)).toEqual({ slug: "camp", id: "r1", focus: "task:t2" });
  });

  it("says so when no tasks are planned, and shows an error when the overview cannot be read", async () => {
    window.location.hash = "#/run?slug=camp";
    setTransport(fakeIpc({ evidence_list: list([]), run_overview: overview({ graph: { tasks: [], waves: 0 } }) }).transport);
    render(<Run />);
    expect(await screen.findByText(S.run.graphEmpty)).toBeTruthy();
    cleanup();
    setTransport(fakeIpc({ evidence_list: list([]), run_overview: new Error("x") }).transport);
    render(<Run />);
    expect(await screen.findByText(S.run.overviewFailed)).toBeTruthy();
  });
});
