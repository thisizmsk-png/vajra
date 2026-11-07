import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { setTransport } from "../src/ipc";
import { Crew } from "../src/screens/crew/Crew";
import { S } from "../strings";
import { fakeIpc } from "./fakeIpc";

afterEach(() => {
  cleanup();
  setTransport();
  window.location.hash = "";
  vi.useRealTimers();
});

const node = (id: string, kind: string, title: string, status: string, children: unknown[] = [], extra: object = {}) =>
  ({ id, kind, title, engine_id: null, status, overlays: [], children, ...extra });

const tree = (backend = "in_progress") =>
  node("lead", "lead", "Lead", "in_progress", [
    node("run", "run", "Run 20261106-1000", "in_progress", [
      node("t1", "task", "Backend", backend, [], { engine_id: "bhima" }),
      node("t2", "task", "Frontend", "pending"),
      node("t3", "worker", "Verifier", "unknown"),
    ]),
  ]);

const open = (replies: Record<string, unknown>) => {
  const f = fakeIpc({ active_runs: { count: 0, runs: [] }, ...replies });
  setTransport(f.transport);
  return f;
};

describe("Crew tree (S2a, US-2.1)", () => {
  it("shows role names with a status dot AND a text status for each node", async () => {
    window.location.hash = "#/crew?slug=camp&id=r1";
    open({ crew: tree() });
    render(<Crew />);
    const t = await screen.findByRole("tree", { name: S.crew.treeLabel });
    const rows = within(t).getAllByRole("treeitem");
    expect(rows.map((r) => r.textContent)).toEqual(
      expect.arrayContaining([expect.stringContaining("Backend"), expect.stringContaining("Frontend")]),
    );
    expect(within(t).getAllByText(S.status.pending).length).toBeGreaterThan(0);
  });

  it("reads a worker with no signal as 'Unknown (no signal)', never idle or done (AC-2.1.5)", async () => {
    window.location.hash = "#/crew?slug=camp&id=r1";
    open({ crew: tree() });
    render(<Crew />);
    const row = (await screen.findByText("Verifier")).closest("[role=treeitem]") as HTMLElement;
    expect(row.textContent).toContain(S.crew.unknownSignal);
    expect(row.textContent).not.toMatch(/idle|done/i);
  });

  it("asks for the run named in the route", async () => {
    window.location.hash = "#/crew?slug=camp&id=r1";
    const f = open({ crew: tree() });
    render(<Crew />);
    await screen.findByRole("tree");
    expect(f.calls.find((c) => c.cmd === "crew")?.args).toEqual({ slug: "camp", runId: "r1" });
  });

  it("without a route run, shows the first active run", async () => {
    const f = open({
      active_runs: { count: 1, runs: [{ stream: "run-s", session_id: "s", run_id: "r9", campaign: "other" }] },
      crew: tree(),
    });
    render(<Crew />);
    await screen.findByRole("tree");
    expect(f.calls.find((c) => c.cmd === "crew")?.args).toEqual({ slug: "other", runId: "r9" });
  });

  it("with no run at all, says so and does not ask for a crew", async () => {
    const f = open({});
    render(<Crew />);
    expect(await screen.findByText(S.crew.noRun)).toBeTruthy();
    expect(f.calls.some((c) => c.cmd === "crew")).toBe(false);
  });

  it("shows an alert when the crew cannot be read", async () => {
    window.location.hash = "#/crew?slug=camp&id=r1";
    open({ crew: new Error("boom") });
    render(<Crew />);
    expect((await screen.findByRole("alert")).textContent).toContain(S.crew.failed);
  });

  it("updates a node's status when a changed event arrives", async () => {
    window.location.hash = "#/crew?slug=camp&id=r1";
    let status = "in_progress";
    const f = open({ crew: () => tree(status) });
    render(<Crew />);
    const row = async () => ((await screen.findByText("Backend")).closest("[role=treeitem]") as HTMLElement).textContent;
    expect(await row()).toContain(S.status.in_progress);
    status = "done";
    act(() => f.emit("changed", {}));
    await waitFor(async () => expect(await row()).toContain(S.status.done));
  });

  it("re-reads on its own every 2 s, because a tasks.json edit raises no event (AC-2.1.2)", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    window.location.hash = "#/crew?slug=camp&id=r1";
    let status = "in_progress";
    open({ crew: () => tree(status) });
    render(<Crew />);
    await screen.findByText("Backend");
    status = "blocked";
    await act(async () => { await vi.advanceTimersByTimeAsync(2100); });
    const row = (await screen.findByText("Backend")).closest("[role=treeitem]") as HTMLElement;
    expect(row.textContent).toContain(S.status.blocked);
  });

  it("keeps the tree open and the selection when it refreshes", async () => {
    window.location.hash = "#/crew?slug=camp&id=r1";
    const f = open({ crew: tree() });
    render(<Crew />);
    const backend = (await screen.findByText("Backend")).closest("[role=treeitem]") as HTMLElement;
    fireEvent.click(backend);
    act(() => f.emit("changed", {}));
    await waitFor(() => expect(f.calls.filter((c) => c.cmd === "crew").length).toBeGreaterThan(1));
    const again = (await screen.findByText("Backend")).closest("[role=treeitem]") as HTMLElement;
    expect(again.getAttribute("aria-selected")).toBe("true");
  });

  it("shows the engine id only in the details of the selected node (AC-2.1.3)", async () => {
    window.location.hash = "#/crew?slug=camp&id=r1";
    open({ crew: tree() });
    render(<Crew />);
    const backend = (await screen.findByText("Backend")).closest("[role=treeitem]") as HTMLElement;
    expect(screen.queryByText("bhima")).toBeNull();
    fireEvent.click(backend);
    const details = await screen.findByRole("region", { name: S.crew.details });
    expect(within(details).getByText("bhima")).toBeTruthy();
    expect(within(screen.getByRole("tree")).queryByText("bhima")).toBeNull();
  });

  it("names an overlay on the node, such as a stop or a pending message", async () => {
    window.location.hash = "#/crew?slug=camp&id=r1";
    const t = tree();
    (t.children[0] as { children: { overlays: string[] }[] }).children[0].overlays = ["stopped by the human"];
    open({ crew: t });
    render(<Crew />);
    const row = (await screen.findByText("Backend")).closest("[role=treeitem]") as HTMLElement;
    expect(row.textContent).toContain(S.crew.overlay["stopped by the human"]);
  });

  it("shows both overlays the core emits for a stopped worker with a queued message, in the core's own strings", async () => {
    window.location.hash = "#/crew?slug=camp&id=r1";
    const t = tree();
    // The exact strings crew/tree.rs puts in `overlays` (see the core test the_crew_tree_shows_message_pending_...).
    (t.children[0] as { children: { overlays: string[] }[] }).children[0].overlays = ["stopped by the human", "message pending"];
    open({ crew: t });
    render(<Crew />);
    const row = (await screen.findByText("Backend")).closest("[role=treeitem]") as HTMLElement;
    expect(row.textContent).toContain(S.crew.overlay["stopped by the human"]);
    expect(row.textContent).toContain(S.crew.overlay["message pending"]);
  });

  it("follows the hash to another run without remounting, dropping the old run's tree and selection", async () => {
    window.location.hash = "#/crew?slug=camp&id=r1";
    const f = open({
      crew: (a?: Record<string, unknown>) =>
        a?.runId === "r1"
          ? node("lead", "lead", "Lead", "in_progress", [node("run", "run", "Run one", "in_progress", [node("t1", "task", "Backend", "in_progress")])])
          : node("lead2", "lead", "Lead", "in_progress", [node("run2", "run", "Run two", "in_progress", [node("u1", "task", "Reviewer", "pending")])]),
    });
    render(<Crew />);
    fireEvent.click((await screen.findByText("Backend")).closest("[role=treeitem]") as HTMLElement);
    expect(await screen.findByRole("region", { name: S.crew.details })).toBeTruthy();
    // Same mounted component; only the hash moves.
    act(() => { window.location.hash = "#/crew?slug=camp2&id=r2"; });
    await screen.findByText("Reviewer");
    expect(screen.queryByText("Backend")).toBeNull();
    expect(f.calls.filter((c) => c.cmd === "crew").map((c) => c.args)).toEqual(
      expect.arrayContaining([{ slug: "camp", runId: "r1" }, { slug: "camp2", runId: "r2" }]),
    );
    // And back again.
    act(() => { window.location.hash = "#/crew?slug=camp&id=r1"; });
    await screen.findByText("Backend");
    expect(screen.queryByText("Reviewer")).toBeNull();
  });

  it("ignores a slow answer for the run the route has already left", async () => {
    window.location.hash = "#/crew?slug=camp&id=r1";
    let release: (v: unknown) => void = () => {};
    const slow = new Promise((r) => { release = r; });
    open({
      crew: (a?: Record<string, unknown>) =>
        a?.runId === "r1" ? slow : node("lead2", "lead", "Lead", "in_progress", [node("u1", "task", "Reviewer", "pending")]),
    });
    render(<Crew />);
    act(() => { window.location.hash = "#/crew?slug=camp2&id=r2"; });
    await screen.findByText("Reviewer");
    await act(async () => { release(node("lead", "lead", "Lead", "in_progress", [node("t1", "task", "Backend", "in_progress")])); await Promise.resolve(); });
    expect(screen.queryByText("Backend")).toBeNull();
    expect(screen.getByText("Reviewer")).toBeTruthy();
  });
});
