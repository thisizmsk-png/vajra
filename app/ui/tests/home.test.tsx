import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { setTransport } from "../src/ipc";
import { Home } from "../src/screens/home/Home";
import { CurrentProject } from "../src/projects/current";
import { validate } from "../src/screens/home/validate";
import { ProjectsProvider } from "../src/projects/store";
import { UNSORTED_ID } from "../src/projects/adapter";
import { S } from "../strings";
import { fakeIpc } from "./fakeIpc";

const rows = (ok: boolean) => ({
  rows: [
    { name: "claude", ok: true, detail: "2.1.0", fix: "" },
    { name: "hooks", ok, detail: ok ? "registered" : "not registered", fix: ok ? "" : "bash install.sh" },
  ],
  start_enabled: ok,
});
const handle = { stream: "s1", session_id: "u", run_id: "20261106-1500", campaign: "deliver-20261106-1500" };
/** What `act_start_run` answers: a receipt, with the run handle when it started. */
const receipt = (state: string, o: Record<string, unknown> = {}) => ({ schema: "vajra.receipt/1", action_id: "act-x", project_id: "p_x", action: "start_run", target: "p_x", expect_generation: null, current_generation: null, state, reason: null, detail: null, retry: "no", rev: 1, ts_ms: 1, ...o });
const started = { receipt: receipt("done"), result: handle };
const camp = (o = {}) => ({ slug: "c1", goal: "ship it", status: "active", passing: 2, total: 5, ...o });

afterEach(() => {
  cleanup();
  setTransport();
  window.location.hash = "";
});

async function open(replies: Record<string, unknown>) {
  const f = fakeIpc({ resume_targets: { targets: {} }, ...replies });
  setTransport(f.transport);
  render(<Home />);
  await waitFor(() => expect(f.calls.map((c) => c.cmd)).toContain("list_campaigns"));
  return f;
}
const fill = (label: string, v: string) => fireEvent.change(screen.getByLabelText(label), { target: { value: v } });

describe("validate", () => {
  const ok = { repo: "/r", ask: "do", base: "main", rounds: "2", maxInFlight: "4" };
  it("accepts a good draft and names each bad field", () => {
    expect(validate(ok)).toEqual({});
    expect(Object.keys(validate({ repo: " ", ask: "", base: "", rounds: "4", maxInFlight: "17" })).sort()).toEqual(["ask", "base", "maxInFlight", "repo", "rounds"]);
    expect(validate({ ...ok, maxInFlight: "1.5" }).maxInFlight).toBeTruthy();
    expect(validate({ ...ok, rounds: "0" }).rounds).toBeTruthy();
  });
});

describe("Home start envelope", () => {
  const T = "p_01JC3H0000000000000000000A";
  const stored = { schema: "vajra.project/1", id: T, name: "Harbor", root: "/src/api", root_key: {}, created: "2026-11-01T09:00:00Z", archived: null, rev: 1 };
  it("an older core without project_for_folder: falls back to the registered root, trailing slash and all", async () => {
    const f = fakeIpc({ preflight: rows(true), list_campaigns: { campaigns: [] }, resume_targets: { targets: {} }, active_runs: { count: 0, runs: [] }, inbox: { count: 0, items: [] }, project_list: { projects: [stored], skipped: 0, truncated: false }, act_start_run: started });
    setTransport(f.transport);
    render(<ProjectsProvider><Home /></ProjectsProvider>);
    await screen.findByText("registered");
    await waitFor(() => expect(f.calls.map((c) => c.cmd)).toContain("project_list"));
    await new Promise((r) => setTimeout(r, 20));
    fill(S.home.repo, "/src/api/");
    fill(S.home.ask, "fix the bug");
    fill(S.home.base, "main");
    fireEvent.click(screen.getByRole("button", { name: S.home.startButton }));
    await waitFor(() => expect(f.calls.some((c) => c.cmd === "act_start_run")).toBe(true));
    const env = (f.calls.find((c) => c.cmd === "act_start_run")!.args as { envelope: { project_id: string } }).envelope;
    expect(env.project_id).toBe(T);
  });
});

describe("Home start envelope: the core's identity", () => {
  const T = "p_01JC3H0000000000000000000A";
  const startFrom = async (folder: string, replies: Record<string, unknown>) => {
    const f = fakeIpc({ preflight: rows(true), list_campaigns: { campaigns: [] }, resume_targets: { targets: {} }, active_runs: { count: 0, runs: [] }, inbox: { count: 0, items: [] }, act_start_run: started, ...replies });
    setTransport(f.transport);
    render(<ProjectsProvider><Home /></ProjectsProvider>);
    await screen.findByText("registered");
    fill(S.home.repo, ` ${folder} `);
    fill(S.home.ask, "fix the bug");
    fill(S.home.base, "main");
    fireEvent.click(screen.getByRole("button", { name: S.home.startButton }));
    await waitFor(() => expect(f.calls.some((c) => c.cmd === "act_start_run")).toBe(true));
    return f;
  };
  const envelope = (f: Awaited<ReturnType<typeof startFrom>>) => (f.calls.find((c) => c.cmd === "act_start_run")!.args as { envelope: { project_id: string } }).envelope;

  it("names the project the core finds for a symlinked or linked-worktree folder, which no literal root match would", async () => {
    const f = await startFrom("/worktrees/api-fix", { project_for_folder: { project_id: T } });
    expect(f.calls.find((c) => c.cmd === "project_for_folder")?.args).toEqual({ path: "/worktrees/api-fix" });
    expect(envelope(f).project_id).toBe(T);
  });

  it("names p_unsorted when the core says no project owns the folder", async () => {
    const f = await startFrom("/elsewhere", { project_for_folder: { project_id: "p_unsorted" } });
    expect(envelope(f).project_id).toBe("p_unsorted");
  });

  it("never sends an id the core did not give: a malformed answer is Unsorted", async () => {
    const f = await startFrom("/elsewhere", { project_for_folder: { project_id: "p_" + "x".repeat(300) } });
    expect(envelope(f).project_id).toBe("p_unsorted");
  });
});

describe("Home", () => {
  it("names the checks in plain words, and shows a check it has no name for as the core sent it", async () => {
    const pre = { rows: [
      { name: "vajra doctor", ok: true, detail: "PASS", fix: "" },
      { name: "hooks registered", ok: true, detail: "yes (6 events)", fix: "" },
      { name: "workflow", ok: true, detail: "installed", fix: "" },
      { name: "jq", ok: true, detail: "/usr/bin/jq", fix: "" },
      { name: "constructor", ok: true, detail: "an odd id", fix: "" },
    ], start_enabled: true };
    const f = fakeIpc({ preflight: pre, list_campaigns: { campaigns: [] }, resume_targets: { targets: {} } });
    setTransport(f.transport);
    render(<Home />);
    expect(await screen.findByText("Engine self-check")).toBeTruthy();
    expect(screen.getByText("Safety hooks")).toBeTruthy();
    expect(screen.getByText("Run workflow")).toBeTruthy();
    expect(screen.getByText("jq")).toBeTruthy();
    expect(screen.getByText("constructor")).toBeTruthy();
    expect(screen.queryByText("vajra doctor")).toBeNull();
    expect(screen.queryByText("hooks registered")).toBeNull();
  });

  it("shows every check, and a failing one with its fix command; Start is disabled (AC-1.3.2)", async () => {
    await open({ preflight: rows(false), list_campaigns: { campaigns: [] } });
    expect(await screen.findByText("bash install.sh")).toBeTruthy();
    expect(screen.getByRole("button", { name: S.home.startButton }).getAttribute("aria-disabled")).toBe("true");
  });

  it("does not start with an empty ask: names the field, calls nothing (AC-1.1.2)", async () => {
    const f = await open({ preflight: rows(true), list_campaigns: { campaigns: [] } });
    await screen.findByText("registered");
    fill(S.home.repo, "/r");
    fireEvent.click(screen.getByRole("button", { name: S.home.startButton }));
    expect(await screen.findByText(S.home.errAsk)).toBeTruthy();
    expect(f.calls.some((c) => c.cmd === "act_start_run")).toBe(false);
  });

  it("starts a run with the entered values and opens the Run view", async () => {
    const f = await open({ preflight: rows(true), list_campaigns: { campaigns: [] }, act_start_run: started });
    await screen.findByText("registered");
    fill(S.home.repo, " /r ");
    fill(S.home.ask, "fix the bug");
    fill(S.home.base, "main");
    fireEvent.click(screen.getByLabelText("2"));
    fireEvent.click(screen.getByRole("button", { name: S.home.startButton }));
    await waitFor(() => expect(window.location.hash).toContain("#/review?"));
    const call = f.calls.find((c) => c.cmd === "act_start_run")!;
    expect(call.args).toMatchObject({ ask: "fix the bug", maxRounds: 2, base: "main", maxInFlight: 4, campaign: null, cwd: "/r" });
    const env = (call.args as { envelope: { action_id: string; project_id: string; expect_generation: number | null } }).envelope;
    expect(env.action_id).toMatch(/^act-[0-9a-f-]{36}$/);
    expect(env).toMatchObject({ project_id: UNSORTED_ID, expect_generation: null }); // /r is no registered project: the core's bucket, never a made-up id
    expect(window.location.hash).toContain("slug=deliver-20261106-1500");
    expect(window.location.hash).toContain("id=20261106-1500");
  });

  it("starts once from Cmd+Return, however often it is pressed while the start is in flight", async () => {
    const f = await open({ preflight: rows(true), list_campaigns: { campaigns: [] }, act_start_run: () => new Promise(() => {}) });
    await screen.findByText("registered");
    fill(S.home.repo, "/r"); fill(S.home.ask, "x");
    const ask = screen.getByLabelText(S.home.ask);
    fireEvent.keyDown(ask, { key: "Enter", metaKey: true });
    fireEvent.keyDown(ask, { key: "Enter", metaKey: true });
    fireEvent.keyDown(ask, { key: "Enter", metaKey: true, repeat: true });
    await waitFor(() => expect(f.calls.some((c) => c.cmd === "act_start_run")).toBe(true));
    expect(f.calls.filter((c) => c.cmd === "act_start_run")).toHaveLength(1);
  });

  it("shows the core's refusal and stays on Home", async () => {
    await open({ preflight: rows(true), list_campaigns: { campaigns: [] }, act_start_run: { receipt: receipt("rejected", { reason: "invalid_args", detail: "base: is not a branch or commit in this repository" }), error: { error: "invalid_args", detail: "base: is not a branch or commit in this repository" } } });
    await screen.findByText("registered");
    fill(S.home.repo, "/r"); fill(S.home.ask, "x");
    fireEvent.click(screen.getByRole("button", { name: S.home.startButton }));
    expect((await screen.findByRole("alert")).textContent).toContain("is not a branch or commit");
    expect(window.location.hash).toBe("");
  });

  it("shows a receipt's own refusal when the core sent no error, and a new click gets a new action id", async () => {
    const f = await open({ preflight: rows(true), list_campaigns: { campaigns: [] }, act_start_run: { receipt: receipt("rejected", { reason: "too_many_actions", detail: "too many recent actions; try again later" }) } });
    await screen.findByText("registered");
    fill(S.home.repo, "/r"); fill(S.home.ask, "x");
    fireEvent.click(screen.getByRole("button", { name: S.home.startButton }));
    expect((await screen.findByRole("alert")).textContent).toContain("too many recent actions");
    fireEvent.click(screen.getByRole("button", { name: S.home.startButton }));
    await waitFor(() => expect(f.calls.filter((c) => c.cmd === "act_start_run")).toHaveLength(2));
    const ids = f.calls.filter((c) => c.cmd === "act_start_run").map((c) => (c.args as { envelope: { action_id: string } }).envelope.action_id);
    expect(ids[0]).not.toBe(ids[1]);
    expect(window.location.hash).toBe("");
  });

  const SID = "0f8fad5b-d9cb-469f-a165-70867728950e";
  const target = { session_id: SID, run_id: "20261106-1000", cwd: "/work/repo" };

  it("resumes the recorded session in its directory, with no repo field needed (claude --resume)", async () => {
    const f = await open({
      preflight: rows(true),
      list_campaigns: { campaigns: [camp(), { slug: "bad", error: "unreadable" }] },
      resume_targets: { targets: { c1: target } },
      resume_run: handle,
    });
    expect(await screen.findByText("2 of 5 passing")).toBeTruthy();
    expect(screen.getByText("unreadable")).toBeTruthy();
    fireEvent.click(await screen.findByRole("button", { name: S.home.resume }));
    await waitFor(() => expect(window.location.hash).toContain("#/review?"));
    expect(f.calls.find((c) => c.cmd === "resume_run")!.args).toEqual({ sessionId: SID, campaign: "c1", cwd: "/work/repo" });
    expect(f.calls.some((c) => c.cmd === "act_start_run")).toBe(false); // never a fresh session
  });

  it("resumes on the provider that ran the campaign, never silently on another", async () => {
    const f = await open({
      preflight: rows(true),
      list_campaigns: { campaigns: [camp()] },
      resume_targets: { targets: { c1: { ...target, provider: "codex" } } },
      resume_run: handle,
    });
    fireEvent.click(await screen.findByRole("button", { name: S.home.resume }));
    await waitFor(() => expect(window.location.hash).toContain("#/review?"));
    expect(f.calls.find((c) => c.cmd === "resume_run")!.args).toEqual({ sessionId: SID, campaign: "c1", cwd: "/work/repo", provider: "codex" });
  });

  it("offers no Resume for a campaign this app did not start, and says why", async () => {
    await open({ preflight: rows(true), list_campaigns: { campaigns: [camp()] }, resume_targets: { targets: {} } });
    expect(await screen.findByText(S.home.resumeElsewhere)).toBeTruthy();
    expect(screen.queryByRole("button", { name: S.home.resume })).toBeNull();
  });

  it("offers no Resume when the targets cannot be read", async () => {
    await open({ preflight: rows(true), list_campaigns: { campaigns: [camp()] }, resume_targets: () => { throw { error: "unknown_command" }; } });
    expect(await screen.findByText(S.home.resumeElsewhere)).toBeTruthy();
    expect(screen.queryByRole("button", { name: S.home.resume })).toBeNull();
  });

  it("shows the core's refusal when resume fails and stays on Home", async () => {
    await open({
      preflight: rows(true), list_campaigns: { campaigns: [camp()] }, resume_targets: { targets: { c1: target } },
      resume_run: () => { throw { error: "run_active", detail: "finish or stop the active run first" }; },
    });
    fireEvent.click(await screen.findByRole("button", { name: S.home.resume }));
    expect((await screen.findByRole("alert")).textContent).toContain("finish or stop the active run");
    expect(window.location.hash).toBe("");
  });

  it("refreshes the campaign list when the core says something changed", async () => {
    let list = [camp()];
    const f = await open({ preflight: rows(true), list_campaigns: () => ({ campaigns: list }) });
    await screen.findByText(/c1/);
    list = [camp(), camp({ slug: "c2", goal: "other" })];
    act(() => f.emit("changed", ["c2"]));
    expect(await screen.findByText(/c2/)).toBeTruthy();
  });

  it("shows a message, not a blank, when the checks cannot run", async () => {
    await open({ preflight: () => { throw { error: "io_error", detail: "no" }; }, list_campaigns: { campaigns: [] } });
    expect((await screen.findByRole("alert")).textContent).toContain(S.home.preflightFailed);
  });
});

describe("Home inside a project (FG-03)", () => {
  const project = { schema: "vajra.project/1", id: "p_1", name: "api", root: "/src/api", created: null, archived: null, rev: 0 };
  const inProject = (replies: Record<string, unknown>) => {
    const f = fakeIpc({ preflight: rows(true), list_campaigns: { campaigns: [camp({ slug: "a1", goal: "api work" }), camp({ slug: "w1", goal: "web work" })] }, ...replies });
    setTransport(f.transport);
    render(<CurrentProject.Provider value={{ project, campaigns: [], focus: null, runId: null }}><Home /></CurrentProject.Provider>);
    return f;
  };
  const list = () => screen.getByRole("region", { name: S.home.campaigns });

  it("Given the folders are still loading, Then the project's list shows loading, not 'no campaigns'", async () => {
    inProject({ resume_targets: () => new Promise(() => {}) });
    await waitFor(() => expect(list().querySelector("[aria-busy=true], [role=status]")).toBeTruthy());
    expect(screen.queryByText(S.home.campaignsEmpty)).toBeNull();
  });

  it("Given the folders cannot be read, Then it says so next to the list and can try again, instead of an empty project", async () => {
    let n = 0;
    const f = inProject({ resume_targets: () => (++n === 1 ? Promise.reject({ error: "io", detail: "runs.jsonl unreadable" }) : { targets: { a1: { session_id: "s", run_id: "r", cwd: "/src/api/" } } }) });
    expect(await screen.findByText(S.home.targetsFailed)).toBeTruthy();
    expect(screen.queryByText(S.home.campaignsEmpty)).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: S.states.retry }));
    expect(await screen.findByText("api work")).toBeTruthy();
    expect(screen.queryByText("web work")).toBeNull();
    expect(f.calls.filter((c) => c.cmd === "resume_targets").length).toBe(2);
  });
});

