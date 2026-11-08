// FG-07 (gap matrix): Given pending items from several projects, when an item arrives or I open it, then the exact
// project/run/worker is focused; approve/reject shows the exact current plan hash and returns a receipt; stale or
// replayed approvals fail; snoozing cannot conceal a new blocker. Notification/click never executes an approval.
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { setTransport } from "../src/ipc";
import { ProjectsProvider } from "../src/projects/store";
import { projectId } from "../src/projects/adapter";
import { attentionText } from "../src/projects/attention";
import { Inbox } from "../src/screens/inbox/Inbox";
import { S } from "../strings";
import { fakeIpc } from "./fakeIpc";

const API = projectId("/src/api");
const WEB = projectId("/src/web");
const SHA = "a".repeat(64);
const plan = (over: Record<string, unknown> = {}) => ({
  id: "plan1", kind: "plan", campaign: "api-1", origin: "app", created: "", age_s: 9, title: "Deploy to staging", body: "SECRET-BODY", actions: ["approve", "reject"],
  source: "Written by the workflow; unsigned", plan_sha256: SHA, action: "deploy",
  plan: { target: "staging", commands: ["make deploy"], rollback: ["make rollback"], health_checks: ["curl /health"], risk: "low" }, invalid: null, ...over,
});
const question = (over: Record<string, unknown> = {}) => ({ id: "q1", kind: "question", campaign: "web-1", origin: "app", created: "", age_s: 5, title: "Which key?", body: "", actions: [], ...over });
let items: unknown[];
const done = (state: string, extra: Record<string, unknown> = {}) => ({ receipt: { schema: "vajra.receipt/1", action_id: "x", project_id: API, action: "approve_plan", target: "plan1", expect_generation: null, current_generation: null, state, reason: null, detail: null, retry: "no", rev: 1, ts_ms: 1, ...extra } });
let act_result: unknown;
let ipc: ReturnType<typeof fakeIpc>;
const mount = (replies: Record<string, unknown> = {}) => {
  ipc = fakeIpc({
    list_campaigns: { campaigns: [
      { slug: "api-1", goal: "Rate limits", status: "active", created: "2026-11-01 09:00", updated: "2026-11-08 09:00" },
      { slug: "web-1", goal: "Login loop", status: "stopped", created: "2026-11-03 09:00", updated: "2026-11-07 09:00" },
    ] },
    resume_targets: { targets: { "api-1": { session_id: "s", run_id: "r1", cwd: "/src/api" }, "web-1": { session_id: "s2", run_id: "r2", cwd: "/src/web" } } },
    active_runs: { count: 0, runs: [] },
    inbox: () => ({ count: items.length, items }),
    act_approve_plan: () => act_result,
    reject_plan: { action_id: "r", state: "rejected" },
    ...replies,
  });
  setTransport(ipc.transport);
  return render(<ProjectsProvider><Inbox /></ProjectsProvider>);
};
const n = (cmd: string) => ipc.calls.filter((c) => c.cmd === cmd).length;
const card = (title: string) => screen.getByRole("article", { name: new RegExp(title) });

beforeEach(() => { items = [plan(), question()]; act_result = done("done"); window.location.hash = ""; });
afterEach(() => { cleanup(); setTransport(); window.location.hash = ""; vi.useRealTimers(); });

describe("Inbox lists what needs you, from every project", () => {
  it("groups items under their project and shows each one's kind", async () => {
    mount();
    expect(await screen.findByRole("heading", { name: "api" })).toBeTruthy();
    expect(screen.getByRole("heading", { name: "web" })).toBeTruthy();
    expect(within(card("Deploy to staging")).getByText(S.inbox.kinds.plan)).toBeTruthy();
    expect(within(card("Which key")).getByText(S.inbox.kinds.question)).toBeTruthy();
  });

  it("says so when nothing needs you", async () => {
    items = [];
    mount();
    expect(await screen.findByText(S.inbox.empty)).toBeTruthy();
  });

  it("does no polling of its own: one inbox read at start", async () => {
    mount();
    await screen.findByText("Deploy to staging");
    expect(n("inbox")).toBe(1);
  });
});

describe("Opening an item focuses the exact project and campaign, and never approves", () => {
  it("a plan opens that project's Review for its campaign", async () => {
    mount();
    await screen.findByText("Deploy to staging");
    fireEvent.click(within(card("Deploy to staging")).getByRole("button", { name: S.inbox.open }));
    expect(window.location.hash).toBe(`#/p/${API}/review?slug=api-1`);
    expect(n("act_approve_plan")).toBe(0);
    expect(n("approve_plan")).toBe(0);
  });

  it("a question in another project opens that project, not the current one", async () => {
    mount();
    await screen.findByText("Which key?");
    fireEvent.click(within(card("Which key")).getByRole("button", { name: S.inbox.open }));
    expect(window.location.hash.startsWith(`#/p/${WEB}/`)).toBe(true);
    expect(window.location.hash).toContain("slug=web-1");
  });
});

describe("Approve and reject", () => {
  it("shows the exact plan hash and what will run before anything is approved", async () => {
    mount();
    await screen.findByText("Deploy to staging");
    const c = card("Deploy to staging");
    expect(within(c).getByText(SHA)).toBeTruthy();
    expect(within(c).getByText("make deploy")).toBeTruthy();
    expect(within(c).getByText(S.inbox.unsigned)).toBeTruthy();
    expect(n("act_approve_plan")).toBe(0);
  });

  it("approves exactly the hash shown, with a project envelope, and shows the receipt", async () => {
    mount();
    await screen.findByText("Deploy to staging");
    fireEvent.click(within(card("Deploy to staging")).getByRole("button", { name: S.inbox.approve }));
    await waitFor(() => expect(n("act_approve_plan")).toBe(1));
    const call = ipc.calls.find((c) => c.cmd === "act_approve_plan")!;
    expect(call.args?.id).toBe("plan1");
    expect(call.args?.planSha256).toBe(SHA);
    const env = call.args?.envelope as { action_id: string; project_id: string; expect_generation: number | null };
    expect(env.project_id).toBe(API);
    expect(env.action_id).toMatch(/^[A-Za-z0-9_-]{8,64}$/);
    expect(await screen.findByText(S.inbox.receipt.done)).toBeTruthy();
  });

  it("refuses a second click while the first is in flight", async () => {
    let finish: (v: unknown) => void = () => {};
    mount({ act_approve_plan: () => new Promise((r) => { finish = r; }) });
    await screen.findByText("Deploy to staging");
    const b = within(card("Deploy to staging")).getByRole("button", { name: S.inbox.approve });
    fireEvent.click(b); fireEvent.click(b);
    expect(n("act_approve_plan")).toBe(1);
    await act(async () => { finish(done("done")); });
  });

  it("says a stale plan changed and does not claim approval", async () => {
    act_result = done("rejected", { reason: "plan_changed", retry: "after_refresh" });
    mount();
    await screen.findByText("Deploy to staging");
    fireEvent.click(within(card("Deploy to staging")).getByRole("button", { name: S.inbox.approve }));
    expect(await screen.findByText(S.inbox.receipt.planChanged)).toBeTruthy();
    expect(screen.queryByText(S.inbox.receipt.done)).toBeNull();
  });

  it("shows a refused, failed or unknown outcome plainly", async () => {
    act_result = done("failed", { reason: "io_error", retry: "safe" });
    mount();
    await screen.findByText("Deploy to staging");
    fireEvent.click(within(card("Deploy to staging")).getByRole("button", { name: S.inbox.approve }));
    expect(await screen.findByText(S.inbox.receipt.failed)).toBeTruthy();
  });

  it("an action the core could not record is shown as not done", async () => {
    act_result = { receipt: null, error: { error: "ledger_unavailable", detail: "disk" } };
    mount();
    await screen.findByText("Deploy to staging");
    fireEvent.click(within(card("Deploy to staging")).getByRole("button", { name: S.inbox.approve }));
    expect(await screen.findByText(S.inbox.receipt.notDone)).toBeTruthy();
  });

  it("offers only reject when the plan cannot be approved", async () => {
    items = [plan({ invalid: "unsupported action", actions: ["reject"] })];
    mount();
    await screen.findByText("Deploy to staging");
    expect(within(card("Deploy to staging")).queryByRole("button", { name: S.inbox.approve })).toBeNull();
    expect(within(card("Deploy to staging")).getByText("unsupported action")).toBeTruthy();
  });

  it("rejects with a reason of 1 to 500 characters", async () => {
    mount();
    await screen.findByText("Deploy to staging");
    const c = card("Deploy to staging");
    const send = within(c).getByRole("button", { name: S.inbox.reject });
    expect(send.getAttribute("aria-disabled")).toBe("true");
    fireEvent.change(within(c).getByLabelText(S.inbox.reason), { target: { value: "x".repeat(501) } });
    expect(send.getAttribute("aria-disabled")).toBe("true");
    fireEvent.change(within(c).getByLabelText(S.inbox.reason), { target: { value: "not yet" } });
    fireEvent.click(send);
    await waitFor(() => expect(n("reject_plan")).toBe(1));
    expect(ipc.calls.find((x) => x.cmd === "reject_plan")?.args).toEqual({ id: "plan1", reason: "not yet" });
  });

  it("reloads the inbox once after an action so the list follows the core", async () => {
    mount();
    await screen.findByText("Deploy to staging");
    items = [question()];
    fireEvent.click(within(card("Deploy to staging")).getByRole("button", { name: S.inbox.approve }));
    await waitFor(() => expect(screen.queryByText("Deploy to staging")).toBeNull());
    expect(n("inbox")).toBe(2);
  });
});

describe("Snooze never hides a new blocker", () => {
  const snooze = (title: string) => fireEvent.click(within(card(title)).getByRole("button", { name: S.inbox.snooze }));

  it("moves a snoozed item out of the way but keeps a visible count", async () => {
    mount();
    await screen.findByText("Which key?");
    snooze("Which key");
    expect(screen.queryByRole("article", { name: /Which key/ })).toBeNull();
    expect(screen.getByText(S.inbox.snoozed(1))).toBeTruthy();
  });

  it("shows a snoozed item again when its plan changes", async () => {
    mount();
    await screen.findByText("Deploy to staging");
    snooze("Deploy to staging");
    expect(screen.queryByRole("article", { name: /Deploy to staging/ })).toBeNull();
    items = [plan({ plan_sha256: "b".repeat(64) }), question()];
    fireEvent.click(screen.getByRole("button", { name: S.inbox.refresh }));
    expect(await screen.findByRole("article", { name: /Deploy to staging/ })).toBeTruthy();
  });

  it("shows a different item that arrives while another is snoozed", async () => {
    mount();
    await screen.findByText("Which key?");
    snooze("Which key");
    items = [plan(), question(), question({ id: "q2", title: "Second blocker" })];
    fireEvent.click(screen.getByRole("button", { name: S.inbox.refresh }));
    expect(await screen.findByText("Second blocker")).toBeTruthy();
  });

  it("brings a snoozed item back after the snooze ends", async () => {
    vi.useFakeTimers({ toFake: ["Date"] });
    mount();
    await act(async () => { await vi.advanceTimersByTimeAsync(0); });
    snooze("Which key");
    expect(screen.queryByRole("article", { name: /Which key/ })).toBeNull();
    vi.setSystemTime(Date.now() + 16 * 60_000);
    fireEvent.click(screen.getByRole("button", { name: S.inbox.refresh }));
    expect(await screen.findByRole("article", { name: /Which key/ })).toBeTruthy();
  });

  it("can be undone from the snoozed list", async () => {
    mount();
    await screen.findByText("Which key?");
    snooze("Which key");
    fireEvent.click(screen.getByRole("button", { name: S.inbox.snoozed(1) }));
    fireEvent.click(screen.getByRole("button", { name: S.inbox.unsnooze }));
    expect(screen.getByRole("article", { name: /Which key/ })).toBeTruthy();
  });
});

describe("Arrival is announced without secrets", () => {
  it("announces a new item by kind and project only", async () => {
    mount();
    await screen.findByText("Which key?");
    items = [plan(), question(), question({ id: "q2", title: "Pay with sk-live-123", body: "sk-live-123" })];
    fireEvent.click(screen.getByRole("button", { name: S.inbox.refresh }));
    await screen.findByText("Pay with sk-live-123");
    const live = screen.getByRole("status");
    expect(live.textContent).toBe(attentionText("question", "web"));
    expect(live.textContent).not.toContain("sk-live");
  });

  it("builds notification text from the kind and project, never the title or body", () => {
    const t = attentionText("plan", "api");
    expect(t).toBe(S.inbox.attention.plan("api"));
    expect(t).not.toContain("SECRET");
  });
});
