import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { setTransport } from "../src/ipc";
import { Home } from "../src/screens/home/Home";
import { CurrentProject } from "../src/projects/current";
import { validate } from "../src/screens/home/validate";
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

describe("Home", () => {
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
    expect(f.calls.some((c) => c.cmd === "start_run")).toBe(false);
  });

  it("starts a run with the entered values and opens the Run view", async () => {
    const f = await open({ preflight: rows(true), list_campaigns: { campaigns: [] }, start_run: handle });
    await screen.findByText("registered");
    fill(S.home.repo, " /r ");
    fill(S.home.ask, "fix the bug");
    fill(S.home.base, "main");
    fireEvent.click(screen.getByLabelText("2"));
    fireEvent.click(screen.getByRole("button", { name: S.home.startButton }));
    await waitFor(() => expect(window.location.hash).toContain("#/review?"));
    const call = f.calls.find((c) => c.cmd === "start_run")!;
    expect(call.args).toMatchObject({ ask: "fix the bug", maxRounds: 2, base: "main", maxInFlight: 4, campaign: null, cwd: "/r" });
    expect(window.location.hash).toContain("slug=deliver-20261106-1500");
    expect(window.location.hash).toContain("id=20261106-1500");
  });

  it("starts once from Cmd+Return, however often it is pressed while the start is in flight", async () => {
    const f = await open({ preflight: rows(true), list_campaigns: { campaigns: [] }, start_run: () => new Promise(() => {}) });
    await screen.findByText("registered");
    fill(S.home.repo, "/r"); fill(S.home.ask, "x");
    const ask = screen.getByLabelText(S.home.ask);
    fireEvent.keyDown(ask, { key: "Enter", metaKey: true });
    fireEvent.keyDown(ask, { key: "Enter", metaKey: true });
    fireEvent.keyDown(ask, { key: "Enter", metaKey: true, repeat: true });
    await waitFor(() => expect(f.calls.some((c) => c.cmd === "start_run")).toBe(true));
    expect(f.calls.filter((c) => c.cmd === "start_run")).toHaveLength(1);
  });

  it("shows the core's refusal and stays on Home", async () => {
    await open({ preflight: rows(true), list_campaigns: { campaigns: [] }, start_run: () => { throw { error: "invalid_input", detail: "base: is not a branch or commit in this repository" }; } });
    await screen.findByText("registered");
    fill(S.home.repo, "/r"); fill(S.home.ask, "x");
    fireEvent.click(screen.getByRole("button", { name: S.home.startButton }));
    expect((await screen.findByRole("alert")).textContent).toContain("is not a branch or commit");
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
    expect(f.calls.some((c) => c.cmd === "start_run")).toBe(false); // never a fresh session
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

