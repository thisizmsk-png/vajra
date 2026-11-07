import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { setTransport } from "../src/ipc";
import { Crew } from "../src/screens/crew/Crew";
import { History } from "../src/screens/history/History";
import { Settings } from "../src/screens/settings/Settings";
import { REGISTRY } from "../src/screens/registry";
import { problemDetail } from "../src/screens/states";
import { S } from "../strings";
import { fakeIpc } from "./fakeIpc";

afterEach(() => {
  cleanup();
  setTransport();
  window.location.hash = "";
});

const node = (id: string, kind: string, title: string, status: string, children: unknown[] = []) =>
  ({ id, kind, title, engine_id: null, status, overlays: [], children });

describe("problem text", () => {
  it("shows the core's own answer, and plain words when the core did not answer at all", () => {
    expect(problemDetail({ error: "io_error", detail: "disk full" })).toBe("io_error: disk full");
    expect(problemDetail("command crew failed")).toBe(S.states.noAnswer);
    expect(problemDetail(new Error("boom"))).toBe(S.states.noAnswer);
  });
});

describe("Crew states", () => {
  it("with no run, offers Home and History instead of a blank page", async () => {
    setTransport(fakeIpc({ active_runs: { count: 0, runs: [] } }).transport);
    render(<Crew />);
    expect(await screen.findByText(S.crew.noRun)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: S.states.openHistory }));
    expect(window.location.hash).toBe("#/history");
  });

  it("when the core does not answer, says so and recovers on Try again", async () => {
    window.location.hash = "#/crew?slug=camp&id=r1";
    let up = false;
    setTransport(fakeIpc({
      active_runs: { count: 0, runs: [] },
      crew: () => { if (!up) throw "command crew failed"; return node("lead", "lead", "Lead", "in_progress"); },
    }).transport);
    render(<Crew />);
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain(S.crew.failed);
    expect(alert.textContent).toContain(S.states.noAnswer);
    fireEvent.click(screen.getByRole("button", { name: S.states.retry }));
    expect((await screen.findByText(/^Tried again at /)).getAttribute("role")).toBe("status");
    up = true;
    fireEvent.click(screen.getByRole("button", { name: S.states.retry }));
    expect(await screen.findByRole("tree", { name: S.crew.treeLabel })).toBeTruthy();
    expect(screen.queryByRole("alert")).toBeNull();
  });
});

describe("History states", () => {
  it("with no campaigns, says what will appear and offers Home", async () => {
    setTransport(fakeIpc({ list_campaigns: { campaigns: [] } }).transport);
    render(<History />);
    expect(await screen.findByText(S.history.emptyHint)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: S.states.goHome }));
    expect(window.location.hash).toBe("#/home");
  });

  it("shows loading rows until the campaigns arrive", async () => {
    let answer: (v: unknown) => void = () => {};
    setTransport(fakeIpc({ list_campaigns: () => new Promise((r) => { answer = r; }) }).transport);
    render(<History />);
    expect(screen.getByRole("status", { name: S.states.loading })).toBeTruthy();
    answer({ campaigns: [] });
    await waitFor(() => expect(screen.queryByRole("status", { name: S.states.loading })).toBeNull());
  });
});

describe("Check again", () => {
  it("says when the check finished, so a re-check that finds nothing new is not a dead click", async () => {
    let calls = 0;
    setTransport(fakeIpc({ preflight: () => { calls++; return { start_enabled: true, rows: [] }; } }).transport);
    render(<Settings />);
    await waitFor(() => expect(screen.getByRole("status").textContent).toMatch(/^Checked at /));
    fireEvent.click(screen.getAllByRole("button", { name: S.settings.recheck })[0]);
    await waitFor(() => expect(calls).toBe(2));
    expect(screen.getByRole("status").textContent).toMatch(/^Checked at /);
  });
});

describe("Screens not built yet", () => {
  it("the Run view right after a start points at the crew for that run", () => {
    window.location.hash = "#/run?slug=camp&id=r1";
    const Run = REGISTRY.run;
    render(<Run />);
    expect(screen.getByText(S.states.runStarted)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: S.states.openCrew }));
    expect(window.location.hash).toBe("#/crew?slug=camp&id=r1");
  });

  it("without a run, it says so and offers Home", () => {
    const Inbox = REGISTRY.inbox;
    render(<Inbox />);
    expect(screen.getByText(S.shell.notBuilt)).toBeTruthy();
    expect(screen.getByRole("button", { name: S.states.goHome })).toBeTruthy();
  });
});
