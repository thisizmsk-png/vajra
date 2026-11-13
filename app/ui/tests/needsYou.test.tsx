// The Needs-you queue (W4 slice 1): only decisions, questions and blocked tasks land in it; the ping rule is shown in
// words and can be changed there; every ping says why it pinged, and an arrival the rule keeps quiet says why not.
// The rule decides only when you are pinged: it cannot start, approve or dismiss anything.
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { setTransport } from "../src/ipc";
import { PingHost } from "../src/projects/PingHost";
import { PreferencesProvider } from "../src/preferences";
import { ProjectsProvider } from "../src/projects/store";
import { Inbox } from "../src/screens/inbox/Inbox";
import { DEFAULT_PING, DEFAULT_PREFERENCES } from "../src/preferenceContract";
import type { PingRule } from "../src/preferenceContract";
import { S } from "../strings";
import { fakeIpc } from "./fakeIpc";

const API = "p_01JC3H0000000000000000000A";
const stored = { schema: "vajra.project/1", id: API, name: "api", root: "/src/api", root_key: {}, created: "2026-11-01T09:00:00Z", archived: null, rev: 1 };
const proposal = { id: "proposal:1", kind: "run_proposal", origin: "app", created: "", age_s: 3, title: "Start a run?", body: "", ask: "Add per-key rate limits",
  base: "main", rounds: 2, project_id: API, project: "/src/api", generation: 1, proposal_sha256: "ab".repeat(32), actions: ["start", "dismiss"] };
const question = { id: "q1", kind: "question", campaign: "api-1", origin: "app", project_id: API, created: "", age_s: 5, title: "Which key?", body: "", actions: [] };
const view = (rev: number, ping: PingRule) => {
  const prefs = { ...DEFAULT_PREFERENCES, ping };
  return { schema: "vajra.preferences/1", rev, project_id: null, caps: { active_runs: 1, workers_per_run: 16, editable: false },
    global: prefs, overrides: {}, effective: prefs, sources: Object.fromEntries(Object.keys(prefs).map((k) => [k, "global"])) };
};

function mount(items: unknown[], rule: PingRule = DEFAULT_PING) {
  let rev = 0;
  let current = rule;
  const ipc = fakeIpc({
    list_campaigns: { campaigns: [{ slug: "api-1", goal: "Rate limits", status: "active", created: "2026-11-01 09:00", updated: "2026-11-08 09:00" }] },
    resume_targets: { targets: { "api-1": { session_id: "s", run_id: "r1", cwd: "/src/api" } } },
    active_runs: { count: 0, runs: [] },
    project_list: { projects: [stored], skipped: 0, truncated: false },
    inbox: { count: items.length, items },
    preferences_read: () => view(rev, current),
    act_preferences_update: (a?: Record<string, unknown>) => {
      current = (a?.patch as { ping: PingRule }).ping;
      rev++;
      return { receipt: { action_id: (a?.envelope as { action_id: string }).action_id, state: "done" }, result: view(rev, current) };
    },
  });
  setTransport(ipc.transport);
  render(<ProjectsProvider><PreferencesProvider projectId={null}><PingHost><Inbox /></PingHost></PreferencesProvider></ProjectsProvider>);
  return ipc;
}

afterEach(() => { cleanup(); setTransport(); window.location.hash = ""; vi.useRealTimers(); });

describe("The Needs-you queue", () => {
  it("lists a run the Director-General proposes as a decision, and sends you to the Director-General to start it", async () => {
    const ipc = mount([proposal, question]);
    const card = await screen.findByRole("article", { name: "Start a run?" });
    expect(within(card).getByText(S.inbox.proposalKind)).toBeTruthy();
    expect(within(card).getByText("Add per-key rate limits")).toBeTruthy();
    // There is no Start here: the plan's threads, models and checks are on the Director-General view.
    expect(within(card).queryByRole("button", { name: S.dg.start })).toBeNull();
    fireEvent.click(within(card).getByRole("button", { name: S.inbox.openProposal }));
    expect(window.location.hash).toBe(`#/p/${API}/orch`);
    expect(ipc.calls.some((c) => c.cmd === "start_proposal")).toBe(false);
  });

  it("says nothing needs you only when no proposal waits either", async () => {
    mount([proposal]);
    await screen.findByRole("article", { name: "Start a run?" });
    expect(screen.queryByText(S.inbox.empty)).toBeNull();
  });
});

describe("The ping rule, visible and editable", () => {
  it("says the rule in words", async () => {
    mount([]);
    expect(await screen.findByText(S.inbox.rule.summary(6, 2, ["decisions", "questions", "blocked"]))).toBeTruthy();
  });

  it("Change opens it inline, and Save writes only the ping rule, globally, through the receipt-checked action", async () => {
    const ipc = mount([]);
    fireEvent.click(await screen.findByRole("button", { name: S.inbox.rule.edit }));
    fireEvent.change(screen.getByLabelText(S.inbox.rule.perHour), { target: { value: "2" } });
    fireEvent.change(screen.getByLabelText(S.inbox.rule.batch), { target: { value: "0" } });
    fireEvent.click(screen.getByRole("switch", { name: S.ping.groupLabels.questions }));
    fireEvent.click(screen.getByRole("button", { name: S.inbox.rule.save }));
    expect(await screen.findByText(S.inbox.rule.saved)).toBeTruthy();
    const call = ipc.calls.find((c) => c.cmd === "act_preferences_update")!;
    expect(call.args?.patch).toEqual({ ping: { per_hour: 2, batch_minutes: 0, decisions: true, questions: false, blocked: true } });
    expect((call.args?.envelope as { project_id: string }).project_id).toBe("global");
    expect(screen.getByText(S.inbox.rule.summary(2, 0, ["decisions", "blocked"]))).toBeTruthy();
  });

  it("Cancel leaves the rule as it was and sends nothing", async () => {
    const ipc = mount([]);
    fireEvent.click(await screen.findByRole("button", { name: S.inbox.rule.edit }));
    fireEvent.change(screen.getByLabelText(S.inbox.rule.perHour), { target: { value: "1" } });
    fireEvent.click(screen.getByRole("button", { name: S.cancel }));
    expect(ipc.calls.some((c) => c.cmd === "act_preferences_update")).toBe(false);
    expect(screen.getByText(S.inbox.rule.summary(6, 2, ["decisions", "questions", "blocked"]))).toBeTruthy();
  });
});

describe("Every ping says why", () => {
  it("Given arrivals inside the batch window, Then one ping leaves when it ends, with why, and the queue's log has it", async () => {
    const ipc = mount([]);
    await screen.findByText(S.inbox.rule.summary(6, 2, ["decisions", "questions", "blocked"]));
    vi.useFakeTimers({ shouldAdvanceTime: false, toFake: ["setTimeout", "clearTimeout", "Date"] });
    act(() => {
      for (let i = 0; i < 5; i++) ipc.emit("inbox_new", { project_id: API, kind: "blocked", count: 1 });
    });
    expect(screen.queryByText(/things need you/)).toBeNull();
    await act(async () => { vi.advanceTimersByTime(2 * 60_000); });
    vi.useRealTimers();
    const text = "5 things need you: 5 blocked tasks in api";
    const why = "Pinged because your rule is on for blocked tasks. Grouped: they arrived within 2 min of the first.";
    // Once as the toast, once in the log.
    await waitFor(() => expect(screen.getAllByText(text)).toHaveLength(2));
    expect(screen.getAllByText(why)).toHaveLength(2);
  });

  it("Given a kind the rule turns off, Then no toast shows and the log says why it did not ping", async () => {
    const ipc = mount([], { ...DEFAULT_PING, questions: false, batch_minutes: 0 });
    await screen.findByText(S.inbox.rule.summary(6, 0, ["decisions", "blocked"]));
    act(() => { ipc.emit("inbox_new", { project_id: API, kind: "question", count: 1 }); });
    expect(await screen.findByText(S.ping.why.muted("questions"))).toBeTruthy();
    expect(screen.getAllByText(S.ping.one.questions("api"))).toHaveLength(1);
    expect(document.querySelector(".v-toast")).toBeNull();
  });

  it("never puts an item's own text in a ping: the event carries only a project, a kind and a count", async () => {
    const ipc = mount([], { ...DEFAULT_PING, batch_minutes: 0 });
    await screen.findByText(S.inbox.rule.summary(6, 0, ["decisions", "questions", "blocked"]));
    act(() => { ipc.emit("inbox_new", { project_id: API, kind: "plan", count: 1, title: "sk-live-123" }); });
    await waitFor(() => expect(document.querySelector(".v-toast")).not.toBeNull());
    expect(document.querySelector(".v-toast")!.textContent).not.toContain("sk-live");
    expect(document.querySelector(".v-toast")!.textContent).toContain(S.ping.one.decisions("api"));
  });
});
