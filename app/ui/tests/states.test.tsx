import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { setTransport } from "../src/ipc";
import { Crew } from "../src/screens/crew/Crew";
import { History } from "../src/screens/history/History";
import { Settings } from "../src/screens/settings/Settings";
import { REGISTRY } from "../src/screens/registry";
import { clock, problemDetail } from "../src/screens/states";
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
    expect(window.location.hash).toBe("#/orch");
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
  it("writes the time without a padded hour, and with seconds so two checks in one minute read differently", () => {
    const a = clock(new Date(2026, 10, 12, 2, 3, 34));
    expect(a).toContain("2:03:34");
    expect(a.startsWith("0")).toBe(false);
    expect(clock(new Date(2026, 10, 12, 2, 3, 35))).not.toBe(a);
  });

  it("says when the check finished, so a re-check that finds nothing new is not a dead click", async () => {
    let calls = 0;
    setTransport(fakeIpc({ preflight: () => { calls++; return { start_enabled: true, rows: [] }; }, provider_list: { schema: "vajra.providers/1", providers: [] }, setup_list: { schema: "vajra.harness_setup/1", rows: [], engine: { installed: true, command: "engine setup command" } } }).transport);
    render(<Settings />);
    await waitFor(() => expect(screen.getByRole("status").textContent).toMatch(/^Checked at /));
    fireEvent.click(screen.getAllByRole("button", { name: S.settings.recheck })[0]);
    await waitFor(() => expect(calls).toBe(2));
    expect(screen.getByRole("status").textContent).toMatch(/^Checked at /);
  });
});

describe("Review view", () => {
  it("right after a start opens on that run's evidence", async () => {
    window.location.hash = "#/run?slug=camp&id=r1";
    const f = fakeIpc({ evidence_list: { slug: "camp", stale: false, rows: [] } });
    setTransport(f.transport);
    const Run = REGISTRY.review;
    render(<Run />);
    await waitFor(() => expect(f.calls.some((c) => c.cmd === "evidence_list" && c.args?.slug === "camp")).toBe(true));
    expect(await screen.findByText(S.run.none)).toBeTruthy();
  });

  it("every view the router knows has a component: no placeholder screens are left", () => {
    for (const v of ["orch", "board", "review", "worker", "crew", "history", "settings"] as const) expect(typeof REGISTRY[v]).toBe("function");
  });
});
