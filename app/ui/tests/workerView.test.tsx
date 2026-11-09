// The worker tab (shell B): only the newest crew answer for the route's run is shown. An older answer, for the same
// run (polls and events out of order) or for the run the tab showed before, never lands over a newer one, whether it
// answers or fails.
import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { setTransport } from "../src/ipc";
import type { CrewNodeDto } from "../src/ipc";
import { WorkerView } from "../src/screens/worker/WorkerView";
import { S } from "../strings";
import { fakeIpc } from "./fakeIpc";

afterEach(() => { cleanup(); setTransport(); window.location.hash = ""; });

const tree = (status: string): CrewNodeDto => ({
  id: "run:r", kind: "run", title: "run", status: "in_progress", children: [
    { id: "task:t1", kind: "task", title: "Token bucket", status: "in_progress", children: [
      { id: "agent:backend", kind: "agent", title: "Backend", role: "Backend", status, children: [] } as unknown as CrewNodeDto,
    ] } as unknown as CrewNodeDto,
  ],
} as unknown as CrewNodeDto);

describe("WorkerView", () => {
  it("Given two loads of one run, When the older answers last, Then the newer worker state stays", async () => {
    let n = 0;
    let answerOld: (v: unknown) => void = () => {};
    window.location.hash = "#/worker?slug=c1&id=r1&worker=backend";
    const f = fakeIpc({ crew: () => (++n === 1 ? new Promise((r) => { answerOld = r; }) : tree("in_progress")) });
    setTransport(f.transport);
    render(<WorkerView />);
    await waitFor(() => expect(n).toBe(1));
    act(() => { f.emit("changed", {}); });
    expect((await screen.findAllByText(S.status.in_progress)).length).toBeGreaterThan(0);
    await act(async () => { answerOld(tree("blocked")); });
    expect(screen.queryAllByText(S.status.blocked)).toHaveLength(0);
    expect(screen.getAllByText(S.status.in_progress).length).toBeGreaterThan(0);
  });

  it("Given the tab moved to another run, When the previous run answers last, Then the new run's worker stays", async () => {
    let answerOld: (v: unknown) => void = () => {};
    window.location.hash = "#/worker?slug=c1&id=r1&worker=backend";
    const f = fakeIpc({ crew: (a?: Record<string, unknown>) => (a?.runId === "r1" ? new Promise((r) => { answerOld = r; }) : tree("in_progress")) });
    setTransport(f.transport);
    render(<WorkerView />);
    await waitFor(() => expect(f.calls.some((c) => c.args?.runId === "r1")).toBe(true));
    act(() => { window.location.hash = "#/worker?slug=c2&id=r2&worker=backend"; });
    expect((await screen.findAllByText(S.status.in_progress)).length).toBeGreaterThan(0);
    await act(async () => { answerOld(tree("blocked")); });
    expect(screen.queryAllByText(S.status.blocked)).toHaveLength(0);
  });

  it("Given the tab moved to another run that is still loading, When the previous run's lookup fails, Then it keeps loading instead of showing that failure", async () => {
    let failOld: (e: unknown) => void = () => {};
    window.location.hash = "#/worker?slug=c1&id=r1&worker=backend";
    const f = fakeIpc({ crew: (a?: Record<string, unknown>) => (a?.runId === "r1" ? new Promise((_, rej) => { failOld = rej; }) : new Promise(() => {})) });
    setTransport(f.transport);
    render(<WorkerView />);
    await waitFor(() => expect(f.calls.some((c) => c.args?.runId === "r1")).toBe(true));
    act(() => { window.location.hash = "#/worker?slug=c2&id=r2&worker=backend"; });
    await waitFor(() => expect(f.calls.some((c) => c.args?.runId === "r2")).toBe(true));
    await act(async () => { failOld({ error: "io", detail: "gone" }); });
    expect(screen.queryByText(S.crew.failed)).toBeNull();
    expect(screen.getByRole("status", { name: S.states.loading })).toBeTruthy();
  });
});
