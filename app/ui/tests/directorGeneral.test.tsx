// FG-02 (ADR-15 §2): the project's Director-General conversation. You ask; it answers on its stream; it proposes runs
// and only a human starts them (D4). Start and Dismiss show what happened: pending, done, refused with the reason, or
// "check before retrying" when the answer was lost (§3.2). Never "done" before the core says so.
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { setTransport } from "../src/ipc";
import type { InboxItem } from "../src/ipc";
import { CurrentProject } from "../src/projects/current";
import type { Current } from "../src/projects/current";
import { DirectorGeneral } from "../src/screens/orch/DirectorGeneral";
import { turns } from "../src/screens/orch/transcript";
import { S } from "../strings";
import { fakeIpc } from "./fakeIpc";

const STREAM = "orchestrator-p_X-g3";
const status = (o: Record<string, unknown> = {}) => ({ available: true, busy: false, name: "Director-General", state: "idle", reason: "waiting for a question", project_id: "p_X", generation: 3, stream: STREAM, ...o });
const win = (texts: string[]) => ({ lines: texts.map((text, i) => ({ seq: i + 1, text })), head_seq: texts.length, first_seq: 1, at_start: true, at_end: true, gap: false });
const proposal = (o: Partial<InboxItem> = {}): InboxItem => ({
  id: "proposal:a1", kind: "run_proposal", origin: "app", created: "", age_s: 5, title: "Start a run?", body: "Add rate limits", actions: ["start", "dismiss"],
  ask: "Add per-key rate limits to the public API", base: "main", rounds: 2, project_id: "p_X", project: "/src/api", generation: 3, proposal_sha256: "ab".repeat(32), ...o,
});
const project = { schema: "vajra.project/1", id: "p_1", name: "api", root: "/src/api", created: null, archived: null, rev: 0 };
const cur = (o: Partial<Current> = {}): Current => ({ project, campaigns: [], focus: "c1", runId: null, dgRun: "20261106-2140", proposals: [], ...o });

function mount(replies: Record<string, unknown>, c: Current = cur()) {
  const f = fakeIpc({ lead_status: status(), stream_window: win(["you\tWhat is left?", "lead\tTwo tasks: the limiter and the docs.", "tool\tRead src/limits.ts"]), ...replies });
  setTransport(f.transport);
  const r = render(<CurrentProject.Provider value={c}><DirectorGeneral /></CurrentProject.Provider>);
  return { ...r, f };
}
const composer = () => screen.getByRole("textbox", { name: S.dg.ask });

afterEach(cleanup);

describe("transcript lines", () => {
  it("groups tagged stream lines into turns, joining a turn's lines", () => {
    expect(turns(["you\tHi", "lead\tHello", "lead\tsecond line", "tool\tRead a.ts", "tool\tGrep x", "error\tboom", "untagged"])).toEqual([
      { role: "you", text: "Hi" },
      { role: "dg", text: "Hello\nsecond line" },
      { role: "tool", text: "Read a.ts\nGrep x" },
      { role: "error", text: "boom" },
      { role: "dg", text: "untagged" },
    ]);
  });
});

describe("Director-General conversation", () => {
  it("Given a conversation, When the view opens, Then it shows your turns, its answers, the generation and its state in words", async () => {
    const { f } = mount({});
    const log = await screen.findByRole("log", { name: S.dg.transcript });
    expect(await within(log).findByText("What is left?")).toBeTruthy();
    expect(within(log).getByText("Two tasks: the limiter and the docs.")).toBeTruthy();
    expect(within(log).getAllByText(S.views.orch).length).toBeGreaterThan(0);
    expect(screen.getByText(S.dg.generation(3))).toBeTruthy();
    expect(screen.getByText(/waiting for a question/i)).toBeTruthy();
    expect(f.calls.find((c) => c.cmd === "lead_status")?.args).toEqual({ runId: "20261106-2140" });
    expect(f.calls.find((c) => c.cmd === "stream_window")?.args).toMatchObject({ stream: STREAM, from: null });
  });

  it("When I ask, Then ask_lead gets the run and my exact text, my question shows at once, and Cancel stops the turn", async () => {
    const { f } = mount({ ask_lead: { ask_id: "k1", state: "running", stream: STREAM, lead_session_id: "s", project_id: "p_X", generation: 3, name: "Director-General" }, cancel_ask: {} });
    await screen.findByRole("log", { name: S.dg.transcript });
    fireEvent.change(composer(), { target: { value: "  Is the limiter done?  " } });
    fireEvent.click(screen.getByRole("button", { name: S.dg.send }));
    await waitFor(() => expect(f.calls.some((c) => c.cmd === "ask_lead")).toBe(true));
    expect(f.calls.find((c) => c.cmd === "ask_lead")?.args).toEqual({ runId: "20261106-2140", text: "Is the limiter done?" });
    expect(screen.getByText("Is the limiter done?")).toBeTruthy();
    fireEvent.click(await screen.findByRole("button", { name: S.dg.cancel }));
    await waitFor(() => expect(f.calls.find((c) => c.cmd === "cancel_ask")?.args).toEqual({ askId: "k1" }));
  });

  it("Cmd+Enter sends from the composer; an empty question is not sent", async () => {
    const { f } = mount({ ask_lead: { ask_id: "k1", state: "running", stream: STREAM, lead_session_id: "s", project_id: "p_X", generation: 3, name: "Director-General" } });
    await screen.findByRole("log", { name: S.dg.transcript });
    fireEvent.keyDown(composer(), { key: "Enter", metaKey: true });
    expect(f.calls.some((c) => c.cmd === "ask_lead")).toBe(false);
    fireEvent.change(composer(), { target: { value: "Status?" } });
    fireEvent.keyDown(composer(), { key: "Enter", metaKey: true });
    await waitFor(() => expect(f.calls.some((c) => c.cmd === "ask_lead")).toBe(true));
  });

  it("Given it is busy, When I ask, Then the refusal is shown in words and my text stays in the box", async () => {
    mount({ ask_lead: () => { throw { error: "lead_busy" }; } });
    await screen.findByRole("log", { name: S.dg.transcript });
    fireEvent.change(composer(), { target: { value: "Again?" } });
    fireEvent.click(screen.getByRole("button", { name: S.dg.send }));
    expect(await screen.findByText(S.dg.busy)).toBeTruthy();
    expect((composer() as HTMLTextAreaElement).value).toBe("Again?");
  });

  it("New conversation starts the next generation on an empty transcript, and is off while it is thinking", async () => {
    const { f } = mount({ new_conversation: { project_id: "p_X", generation: 4, stream: "orchestrator-p_X-g4" } });
    await screen.findByRole("log", { name: S.dg.transcript });
    fireEvent.click(screen.getByRole("button", { name: S.dg.newConversation }));
    expect(await screen.findByText(S.dg.generation(4))).toBeTruthy();
    expect(f.calls.find((c) => c.cmd === "new_conversation")?.args).toEqual({ runId: "20261106-2140" });
    cleanup();
    mount({ lead_status: status({ busy: true, state: "thinking", reason: "answering a question", ask_id: "k9" }) });
    await screen.findByText(/answering a question/i);
    expect(screen.getByRole("button", { name: S.dg.newConversation }).getAttribute("aria-disabled")).toBe("true");
  });

  it("Given it cannot be asked, Then the reason shows and the composer is off", async () => {
    mount({ lead_status: { available: false, busy: false, name: "Director-General", read_only_reason: "Asking the Director-General needs the Claude Code sandbox." } });
    expect(await screen.findByText(/needs the Claude Code sandbox/)).toBeTruthy();
    expect(composer().getAttribute("aria-disabled")).toBe("true");
  });

  it("Given a project with no run yet, Then it says so plainly and offers to start a run yourself", async () => {
    const f = fakeIpc({});
    setTransport(f.transport);
    render(<CurrentProject.Provider value={cur({ dgRun: null })}><DirectorGeneral /></CurrentProject.Provider>);
    expect(screen.getByText(S.dg.noRun)).toBeTruthy();
    expect(screen.getByRole("link", { name: S.dg.startYourself })).toBeTruthy();
    expect(f.calls).toEqual([]);
  });
});

describe("run proposals (D4: only a human starts a run)", () => {
  const card = () => screen.getByRole("article", { name: S.dg.proposal });

  it("shows the exact ask it proposed, with base and rounds, and Start sends the hash of what was shown", async () => {
    const { f } = mount({ start_proposal: { action_id: "a9", state: "started", run: {} } }, cur({ proposals: [proposal()] }));
    await screen.findByRole("log", { name: S.dg.transcript });
    expect(within(card()).getByText("Add per-key rate limits to the public API")).toBeTruthy();
    expect(within(card()).getByText("main")).toBeTruthy();
    expect(within(card()).getByText("2")).toBeTruthy();
    fireEvent.click(within(card()).getByRole("button", { name: S.dg.start }));
    expect(within(card()).getByRole("status").textContent).toBe(S.dg.receipt.pending);
    await waitFor(() => expect(within(card()).getByRole("status").textContent).toBe(S.dg.receipt.started));
    expect(f.calls.find((c) => c.cmd === "start_proposal")?.args).toEqual({ id: "proposal:a1", proposalSha256: "ab".repeat(32) });
  });

  it("a proposal that changed after it was shown is refused with the reason, and nothing retries by itself", async () => {
    const { f } = mount({ start_proposal: () => { throw { error: "proposal_changed" }; } }, cur({ proposals: [proposal()] }));
    await screen.findByRole("log", { name: S.dg.transcript });
    fireEvent.click(within(card()).getByRole("button", { name: S.dg.start }));
    await waitFor(() => expect(within(card()).getByRole("status").textContent).toBe(S.dg.receipt.changed));
    expect(f.calls.filter((c) => c.cmd === "start_proposal")).toHaveLength(1);
  });

  it("a lost answer reads as unknown: check before retrying", async () => {
    mount({ start_proposal: () => new Error("socket closed") }, cur({ proposals: [proposal()] }));
    await screen.findByRole("log", { name: S.dg.transcript });
    fireEvent.click(within(card()).getByRole("button", { name: S.dg.start }));
    await waitFor(() => expect(within(card()).getByRole("status").textContent).toBe(S.dg.receipt.unknown));
  });

  it("Dismiss sends dismiss_proposal; Start and Dismiss are off while one is in flight", async () => {
    let release: (v: unknown) => void = () => {};
    const { f } = mount({ dismiss_proposal: () => new Promise((r) => { release = r; }) }, cur({ proposals: [proposal()] }));
    await screen.findByRole("log", { name: S.dg.transcript });
    fireEvent.click(within(card()).getByRole("button", { name: S.dg.dismiss }));
    expect(within(card()).getByRole("button", { name: S.dg.start }).getAttribute("aria-disabled")).toBe("true");
    await act(async () => { release({ action_id: "a2", state: "dismissed" }); });
    expect(within(card()).getByRole("status").textContent).toBe(S.dg.receipt.dismissed);
    expect(f.calls.find((c) => c.cmd === "dismiss_proposal")?.args).toEqual({ id: "proposal:a1" });
  });
});
