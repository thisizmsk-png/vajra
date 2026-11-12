// The shell's project store: a newer load always wins. An older load that finishes late, whether its crew lookup
// answered or failed, must not publish its campaigns, inbox or crew over the newer one.
import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
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

  it("Given a loaded store, When the inbox changes while resume_targets fails, Then the shared inbox still updates", async () => {
    const item = (id: string) => ({ id, kind: "task", campaign: "c", title: id, detail: "", status: "blocked", since: "2026-11-01T00:00:00Z" });
    let n = 0;
    const f = fakeIpc({
      list_campaigns: { campaigns: [{ slug: "c", goal: "g", status: "done" }] },
      resume_targets: () => { if (++n > 1) throw new Error("resume failed"); return { targets: {} }; },
      active_runs: { runs: [] },
      inbox: () => (n > 1 ? { count: 1, items: [item("task:c.new")] } : { count: 1, items: [item("task:c.old")] }),
    });
    setTransport(f.transport);
    render(<ProjectsProvider><Probe /></ProjectsProvider>);
    await waitFor(() => expect(view?.inbox.map((i) => i.id)).toEqual(["task:c.old"]));
    await act(async () => { view!.reload(); });
    await waitFor(() => expect(view?.state).toBe("error"));
    expect(view!.inbox.map((i) => i.id)).toEqual(["task:c.new"]);
  });

  it("Given the core's project store, Then a registered folder's project carries its T1 id and name; without the store everything still shows", async () => {
    const T = "p_01JC3H0000000000000000000A";
    const base = {
      list_campaigns: { campaigns: [{ slug: "a1", goal: "g", status: "done" }] },
      resume_targets: { targets: { a1: { session_id: "s", run_id: "r1", cwd: "/src/api" } } },
      active_runs: { runs: [] }, inbox: { count: 0, items: [] },
    };
    setTransport(fakeIpc({ ...base, project_list: { projects: [{ schema: "vajra.project/1", id: T, name: "Harbor API", root: "/src/api", root_key: {}, created: "2026-11-01T09:00:00Z", archived: null, rev: 2 }], skipped: 0, truncated: false } }).transport);
    const r = render(<ProjectsProvider><Probe /></ProjectsProvider>);
    await waitFor(() => expect(view?.state).toBe("ok"));
    expect(view!.projects.map((p) => [p.id, p.name, p.registered])).toEqual([[T, "Harbor API", true]]);
    expect(view!.byCampaign.a1).toBe(T);
    r.unmount();
    setTransport(fakeIpc(base).transport); // an older core: project_list is unknown
    render(<ProjectsProvider><Probe /></ProjectsProvider>);
    await waitFor(() => expect(view?.state).toBe("ok"));
    expect(view!.projects).toHaveLength(1);
    expect(view!.projects[0].registered).toBeUndefined();
  });

  it("Given a run proposal waits, When the inbox loads, Then inboxCampaignCount leaves it out and inboxCount keeps the total", async () => {
    const f = fakeIpc({
      list_campaigns: { campaigns: [] },
      resume_targets: { targets: {} },
      active_runs: { runs: [] },
      inbox: { count: 2, campaign_count: 1, items: [
        { id: "task:c.t1", kind: "task", campaign: "c", title: "t", detail: "", status: "blocked", since: "2026-11-01T00:00:00Z" },
        { id: "proposal:01JC3H0000000000000000PRP1", kind: "run_proposal", title: "Start a run?", project_id: "p_unsorted" },
      ] },
    });
    setTransport(f.transport);
    render(<ProjectsProvider><Probe /></ProjectsProvider>);
    await screen.findByText(/^ok:/);
    expect(view!.inboxCount).toBe(2);
    expect(view!.inboxCampaignCount).toBe(1);
    cleanup();
    // An older core sends no campaign_count: the store says it does not know, rather than passing off the cut list as the total.
    setTransport(fakeIpc({ list_campaigns: { campaigns: [] }, resume_targets: { targets: {} }, active_runs: { runs: [] }, inbox: { count: 40, items: [{ id: "task:c.t1", kind: "task", campaign: "c", title: "t", detail: "", status: "blocked", since: "2026-11-01T00:00:00Z" }] } }).transport);
    render(<ProjectsProvider><Probe /></ProjectsProvider>);
    await waitFor(() => expect(view?.inbox.length).toBe(1));
    expect(view!.inboxCampaignCount).toBeNull();
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

