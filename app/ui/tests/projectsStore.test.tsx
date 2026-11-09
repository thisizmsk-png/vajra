// The shell's project store: a newer load always wins. An older load that finishes late, whether its crew lookup
// answered or failed, must not publish its campaigns, inbox or crew over the newer one.
import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { setTransport } from "../src/ipc";
import { ProjectsProvider, useProjects } from "../src/projects/store";
import type { ProjectsView } from "../src/projects/store";
import { fakeIpc } from "./fakeIpc";

afterEach(cleanup);

let view: ProjectsView | null = null;
function Probe() {
  view = useProjects();
  return <p>{view.state}:{view.campaigns.map((c) => c.slug).join(",")}:{view.crew?.runId ?? "none"}</p>;
}

describe("ProjectsProvider", () => {
  it("Given an old load still waiting on its crew, When a newer load finishes and the old crew lookup then fails, Then the newer load stays", async () => {
    let n = 0;
    let failOld: (e: unknown) => void = () => {};
    const f = fakeIpc({
      list_campaigns: () => ({ campaigns: [{ slug: ++n === 1 ? "old" : "new", goal: "g", status: "done" }] }),
      resume_targets: { targets: {} },
      active_runs: () => (n === 1 ? { runs: [{ run_id: "r-old", campaign: "old" }] } : { runs: [] }),
      inbox: { count: 0, items: [] },
      crew: () => new Promise((_, reject) => { failOld = reject; }),
    });
    setTransport(f.transport);
    render(<ProjectsProvider><Probe /></ProjectsProvider>);
    await act(async () => { await Promise.resolve(); });
    await act(async () => { view!.reload(); });
    expect(await screen.findByText("ok:new:none")).toBeTruthy();
    await act(async () => { failOld(new Error("socket closed")); });
    expect(screen.getByText("ok:new:none")).toBeTruthy();
  });

  it("Given more waiting than the core sends, When the inbox loads, Then inboxCount is the core total, not the cut list", async () => {
    const f = fakeIpc({
      list_campaigns: { campaigns: [] },
      resume_targets: { targets: {} },
      active_runs: { runs: [] },
      inbox: { count: 350, items: [{ id: "task:c.t1", kind: "task", campaign: "c", title: "t", detail: "", status: "blocked", since: "2026-11-01T00:00:00Z" }] },
    });
    setTransport(f.transport);
    render(<ProjectsProvider><Probe /></ProjectsProvider>);
    await screen.findByText(/^ok:/);
    expect(view!.inbox.length).toBe(1);
    expect(view!.inboxCount).toBe(350);
  });
});
