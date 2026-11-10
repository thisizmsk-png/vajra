// The Director-General adapter (dgApi): FG-02c's project-keyed shapes with the FG-11 envelope, over #83's run-keyed
// commands as the interim. These pin what the swap must keep: an action id per click, reused only to retry an unknown
// outcome; refusals and lost answers in the receipt's words; proposals keyed by the core's project id once known; and
// reloads only for this Director-General's own stream, at most once per gap.
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { renderHook } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { setTransport } from "../src/ipc";
import type { InboxItem } from "../src/ipc";
import { CurrentProject } from "../src/projects/current";
import { DirectorGeneral } from "../src/screens/orch/DirectorGeneral";
import { actionId, dg, envelope, failure, proposalsFor, useDgReload } from "../src/screens/orch/dgApi";
import { S } from "../strings";
import { fakeIpc } from "./fakeIpc";

afterEach(() => { cleanup(); vi.useRealTimers(); });

const item = (o: Partial<InboxItem> = {}): InboxItem => ({
  id: "proposal:a1", kind: "run_proposal", origin: "app", created: "", age_s: 1, title: "Start a run?", body: "", actions: ["start", "dismiss"],
  project: "/src/api", project_id: "p_X", proposal_sha256: "ab".repeat(32), ...o,
});

describe("envelope and outcomes", () => {
  it("makes a fresh, URL-safe action id per call and carries the project and the generation shown", () => {
    const a = actionId(), b = actionId();
    expect(a).toMatch(/^a_[0-9a-f]{24}$/);
    expect(a).not.toBe(b);
    expect(envelope({ projectId: "p_X", runId: "r" }, a, 3)).toEqual({ action_id: a, project_id: "p_X", expect_generation: 3 });
  });

  it("says a refusal with its reason, a changed proposal or stale conversation plainly, and a lost answer as unknown", () => {
    expect(failure(new Error("socket closed")).outcome.kind).toBe("unknown");
    expect(failure({ error: "proposal_changed" }).outcome).toEqual({ kind: "refused", text: S.dg.receipt.changed });
    expect(failure({ error: "stale_generation" }).code).toBe("stale_generation");
    expect(failure({ error: "lead_busy" }).outcome.text).toBe(S.dg.busy);
    expect(failure({ error: "denied", detail: "no" }).outcome.kind).toBe("refused");
  });

  it("refuses to start a proposal with no hash rather than starting something unseen", async () => {
    const f = fakeIpc({ start_proposal: {} });
    setTransport(f.transport);
    const r = await dg.startProposal({ projectId: null, runId: "r" }, envelope({ projectId: null, runId: "r" }, actionId(), null), item({ proposal_sha256: undefined }));
    expect(r.kind).toBe("refused");
    expect(f.calls).toEqual([]);
  });

  it("never calls the core without a run to address (interim)", async () => {
    const f = fakeIpc({});
    setTransport(f.transport);
    const t = { projectId: "p_X", runId: null };
    expect((await dg.ask(t, envelope(t, actionId(), 1), "hi")).ok).toBe(false);
    expect((await dg.newConversation(t, envelope(t, actionId(), 1))).ok).toBe(false);
    expect(f.calls).toEqual([]);
  });
});

describe("proposalsFor", () => {
  const inbox = [item(), item({ id: "proposal:b", project: "/src/web", project_id: "p_Y" }), item({ id: "q", kind: "question", campaign: "c" })];
  it("keys by the core's project id once known, even if the folder moved", () => {
    expect(proposalsFor(inbox, "p_X", "/moved/api").map((i) => i.id)).toEqual(["proposal:a1"]);
  });
  it("falls back to the folder (interim), ignoring trailing slashes, and never matches a project with no folder", () => {
    expect(proposalsFor(inbox, null, "/src/web/").map((i) => i.id)).toEqual(["proposal:b"]);
    expect(proposalsFor(inbox, null, null)).toEqual([]);
  });
});

describe("useDgReload", () => {
  it("reloads only for its own stream, at most once per gap, and stops listening on unmount", () => {
    vi.useFakeTimers();
    const f = fakeIpc({});
    setTransport(f.transport);
    const reload = vi.fn();
    const { unmount } = renderHook(() => useDgReload("orchestrator-p_X-g3", reload, 500));
    f.emit("changed", { streams: ["other"] });
    expect(reload).not.toHaveBeenCalled();
    f.emit("changed", { streams: ["orchestrator-p_X-g3"] });
    f.emit("changed", { streams: ["orchestrator-p_X-g3"] });
    f.emit("changed", { streams: ["orchestrator-p_X-g3"] });
    expect(reload).toHaveBeenCalledTimes(1);
    act(() => { vi.advanceTimersByTime(500); });
    expect(reload).toHaveBeenCalledTimes(2);
    unmount();
    expect(f.listenerCount("changed")).toBe(0);
  });
});

describe("Start after an unknown outcome", () => {
  it("retries with the same action id; a settled answer retires it", async () => {
    let n = 0;
    const f = fakeIpc({
      lead_status: { available: true, busy: false, name: "Director-General", state: "idle", reason: "", project_id: "p_X", generation: 3, stream: "orchestrator-p_X-g3" },
      stream_window: { lines: [], head_seq: 0, first_seq: 0, at_start: true, at_end: true, gap: false },
      start_proposal: () => (++n === 1 ? new Error("socket closed") : { state: "started" }),
    });
    setTransport(f.transport);
    const spy = vi.spyOn(crypto, "getRandomValues");
    const project = { schema: "vajra.project/1", id: "p_1", name: "api", root: "/src/api", created: null, archived: null, rev: 0 };
    render(<CurrentProject.Provider value={{ project, campaigns: [], focus: null, runId: null, dgRun: "r1", proposals: [item()] }}><DirectorGeneral /></CurrentProject.Provider>);
    const card = await screen.findByRole("article", { name: S.dg.proposal });
    fireEvent.click(within(card).getByRole("button", { name: S.dg.start }));
    await waitFor(() => expect(within(card).getByRole("status").textContent).toBe(S.dg.receipt.unknown));
    const minted = spy.mock.calls.length;
    fireEvent.click(within(card).getByRole("button", { name: S.dg.start }));
    await waitFor(() => expect(within(card).getByRole("status").textContent).toBe(S.dg.receipt.started));
    expect(spy.mock.calls.length).toBe(minted); // no new id for the retry
    expect(f.calls.filter((c) => c.cmd === "start_proposal")).toHaveLength(2);
    spy.mockRestore();
  });
});
