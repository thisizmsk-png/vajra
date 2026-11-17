// Every agent is a chat you can read and talk to: a worker's stream read as a conversation, its message box on the
// core's steering path with where each message is, and the Lead's tool calls as cards that open a worker's chat.
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { setTransport } from "../src/ipc";
import type { CrewNodeDto } from "../src/ipc";
import { Crew } from "../src/screens/crew/Crew";
import { chatItems, fileDiff, lastLine, stepFile, steerBlocks, testSummary } from "../src/screens/crew/chat";
import { SENT_PER_WORKER, sentStore } from "../src/screens/crew/sentStore";
import { toolCalls } from "../src/screens/orch/transcript";
import { S } from "../strings";
import { fakeIpc } from "./fakeIpc";
import { Live, liveRun } from "./liveRun";

afterEach(() => {
  cleanup();
  setTransport();
  sentStore.clear();
  window.location.hash = "";
  vi.useRealTimers();
});

const L = S.crew.live;
const lines = (texts: string[], from = 1) => texts.map((text, i) => ({ seq: from + i, text }));
/** A relayed message as the core's tailer writes it: every line of the hook's text tagged `[hook] `, empty lines dropped. */
const steer = (id: string, text: string, source = "human", kind = "message") =>
  ["Relayed by Vajra Desktop from the human operator:", `<vajra-steer nonce="[redacted]" source="${source}" kind="${kind}" id="${id}">`, text, "</vajra-steer>"]
    .map((l) => `[hook] ${l}`).join("\n");

describe("a worker's stream as a chat", () => {
  it("reads the Lead's task, the agent's sentences, steps with paths and test counts, and relayed messages", () => {
    const items = chatItems(lines([
      "[user] Build the bucket.",
      "I will read the middleware first.",
      "[tool] Read: /w/src/middleware/rateLimit.ts",
      "[tool] Bash: npm test -- rateLimit",
      "[result] ✓ allows 60 requests",
      "[result] Tests: 1 failed, 2 passed, 3 total",
      ...steer("act_1", "key the bucket by id &lt;not&gt; the raw key", "lead-via-desktop", "redirect").split("\n"),
    ]));
    expect(items.map((i) => i.kind)).toEqual(["lead", "agent", "step", "step", "steer"]);
    expect(items[2]).toMatchObject({ tool: "Read", path: "/w/src/middleware/rateLimit.ts" });
    expect(items[3]).toMatchObject({ tool: "Bash", path: null, test: { passed: 2, failed: 1 } });
    expect(items[4]).toMatchObject({ from: "lead", redirect: true, id: "act_1", text: "key the bucket by id <not> the raw key" });
    expect(lastLine(items)).toBe("Bash npm test -- rateLimit");
  });

  it("finds pass and fail counts in common runners' summaries, and nothing in other output", () => {
    expect(testSummary(["test result: FAILED. 3 passed; 1 failed; 0 ignored"])).toEqual({ passed: 3, failed: 1 });
    expect(testSummary(["  12 passing (40ms)", "  2 failing"])).toEqual({ passed: 0, failed: 2 });
    expect(testSummary(["Tests  36 passed (36)"])).toEqual({ passed: 36, failed: 0 });
    expect(testSummary(["built in 2s"])).toBeNull();
  });

  it("splits several relayed messages in one hook entry, and leaves other hook text a note", () => {
    const two = chatItems(lines(`${steer("a", "one")}\n${steer("b", "two")}`.split("\n")));
    expect(two.map((i) => (i.kind === "steer" ? i.id : i.kind))).toEqual(["a", "b"]);
    expect(steerBlocks(`<vajra-steer source="human" kind="message" id="c">\nhi\n</vajra-steer>`)).toEqual([{ from: "you", redirect: false, id: "c", text: "hi" }]);
    expect(chatItems(lines(["[hook] Remember the guard."])).map((i) => i.kind)).toEqual(["note"]);
  });

  it("places the human's sent message: Sent until its block reaches the worker, then Seen in its place; Not delivered once the worker ends", () => {
    const sent = [{ key: "k", id: "act_9", text: "hurry", redirect: false, state: "sent" as const }];
    const before = chatItems(lines(["Working."]), sent);
    expect(before[before.length - 1]).toMatchObject({ kind: "steer", from: "you", state: "sent", text: "hurry" });
    expect(chatItems(lines(["Working."]), sent, true).pop()).toMatchObject({ state: "ended" });
    const after = chatItems(lines(["Working.", ...steer("act_9", "hurry").split("\n")]), sent);
    expect(after.filter((i) => i.kind === "steer")).toHaveLength(1);
    expect(after[after.length - 1]).toMatchObject({ state: "seen" });
  });

  it("finds one file's part of a diff by the end of an absolute path", () => {
    const d = "diff --git a/src/a.ts b/src/a.ts\n@@ -1 +1 @@\n-x\n+y\ndiff --git a/src/b.ts b/src/b.ts\n@@ -1 +1 @@\n-p\n+q\n";
    expect(fileDiff(d, "/w/src/b.ts")).toContain("+q");
    expect(fileDiff(d, "/w/src/b.ts")).not.toContain("+y");
    expect(fileDiff(d, "/w/src/c.ts")).toBe("");
  });

  it("a message still sending when the worker ends says Not delivered too", () => {
    const sent = [{ key: "k", id: "", text: "hurry", redirect: false, state: "sending" as const }];
    expect(chatItems(lines(["Working."]), sent).pop()).toMatchObject({ state: "sending" });
    expect(chatItems(lines(["Working."]), sent, true).pop()).toMatchObject({ state: "ended" });
  });

  it("matches the exact worktree-relative path before one that only ends the same", () => {
    const d = "diff --git a/web/index.ts b/web/index.ts\n@@ -1 +1 @@\n-w\n+W\ndiff --git a/index.ts b/index.ts\n@@ -1 +1 @@\n-r\n+R\n";
    // Under the worktree: only the exact relative path.
    expect(fileDiff(d, "/w/index.ts", "/w")).toContain("+R");
    expect(fileDiff(d, "/w/web/index.ts", "/w/")).toContain("+W");
    expect(fileDiff(d, "/w/api/index.ts", "/w")).toBe("");
    // No root known: an exact name wins over an ending.
    expect(fileDiff(d, "index.ts")).toContain("+R");
  });

  it("a step for a new file, a cut diff or a file past the cut says so, never No change", () => {
    const one = "diff --git a/src/a.ts b/src/a.ts\n@@ -1 +1 @@\n-x\n+y\n";
    const base = { diff: one, truncated: false };
    expect(stepFile(base, "/w/src/a.ts", "/w")).toEqual({ kind: "diff", diff: one, cut: false });
    // git diff leaves out a file git does not track: it is in the untracked list instead.
    expect(stepFile({ ...base, untracked: ["test/new.test.ts"] }, "/w/test/new.test.ts", "/w")).toEqual({ kind: "new" });
    // The diff stopped inside the file's part: show it, marked as cut.
    expect(stepFile({ diff: one, truncated: true }, "/w/src/a.ts", "/w")).toMatchObject({ kind: "diff", cut: true });
    // The diff stopped before the file, or the untracked list did.
    expect(stepFile({ diff: one, truncated: true }, "/w/src/z.ts", "/w")).toEqual({ kind: "cut" });
    expect(stepFile({ ...base, untracked: [], untracked_truncated: true }, "/w/src/z.ts", "/w")).toEqual({ kind: "cut" });
    expect(stepFile(base, "/w/src/z.ts", "/w")).toEqual({ kind: "none" });
  });

  it("keeps at most SENT_PER_WORKER messages per worker", () => {
    for (let i = 0; i < SENT_PER_WORKER + 5; i++) sentStore.add("agent:x", { key: `${i}`, id: "", text: `${i}`, redirect: false, state: "sent" });
    expect(sentStore.get("agent:x")).toHaveLength(SENT_PER_WORKER);
    expect(sentStore.get("agent:x")[0].text).toBe("5");
  });
});

describe("the Lead's tool calls", () => {
  it("names the worker a message, redirect or stop was for, even from input cut at 200 characters", () => {
    const calls = toolCalls([
      `mcp__vajra-desktop__message_worker {"worker":"agent:a2","text":"use the key id"}`,
      `mcp__vajra-desktop__redirect_worker {"worker":"a3","text":"${"x".repeat(180)}…`,
      `list_workers {}`,
      `result ok`,
    ].join("\n"));
    expect(calls.map((c) => [c.tool, c.worker, c.action])).toEqual([
      ["message_worker", "agent:a2", "message"], ["redirect_worker", "a3", "redirect"], ["list_workers", null, "inspect"], ["result", null, "inspect"],
    ]);
    expect(calls[0].text).toBe("use the key id");
  });
});

const node = (id: string, kind: string, title: string, status: string, children: CrewNodeDto[] = [], extra: Partial<CrewNodeDto> = {}): CrewNodeDto =>
  ({ id, kind, title, engine_id: null, status, overlays: [], children, ...extra });
const tree = node("lead", "lead", "Lead", "in_progress", [
  node("run:r1", "run", "Run r1", "in_progress", [
    node("task:t2", "task", "Backend · t2", "in_progress", [node("agent:a2", "round", "round 1", "in_progress", [], { role: "Backend" })]),
  ]),
]);
const win = (texts: string[]) => ({ lines: lines(texts), head_seq: texts.length, first_seq: 1, at_start: true, at_end: true, gap: false });

describe("talking to a worker", () => {
  const open = (replies: Record<string, unknown>) => {
    const f = fakeIpc({ ...liveRun(), crew: tree, task: { task: { title: "Bucket" } }, worktree_info: { worker: "a2", worktree: "/w", branch: "b", base: null }, ...replies });
    setTransport(f.transport);
    window.location.hash = "#/crew?slug=camp&id=r1&worker=a2";
    render(<Live><Crew /></Live>);
    return f;
  };

  it("sends through the core's control message, shows Sent, then Seen when the worker's transcript has it", async () => {
    let texts = ["Working."];
    const f = open({ stream_window: () => win(texts), tail: () => ({ lines: [], gap: false, head_seq: texts.length, spill_start: null, spill_end: null }), control_message: { action_id: "act_7", state: "queued" } });
    const box = await screen.findByRole("textbox", { name: L.ask("Backend") });
    await waitFor(() => expect(box.getAttribute("aria-disabled")).toBeNull());
    fireEvent.change(box, { target: { value: "key it by id" } });
    fireEvent.click(screen.getByRole("button", { name: L.send }));
    await waitFor(() => expect(f.calls.some((c) => c.cmd === "control_message" && c.args?.worker === "agent:a2" && c.args?.text === "key it by id")).toBe(true));
    const log = screen.getByRole("log", { name: L.label });
    await waitFor(() => expect(within(log).getByText(L.delivery.sent)).toBeTruthy());
    // The hook hands it over: the relayed block is in the worker's transcript now.
    texts = ["Working.", ...steer("act_7", "key it by id").split("\n")];
    await waitFor(() => expect(within(log).getByText(L.delivery.seen)).toBeTruthy(), { timeout: 3000 });
    expect(within(log).getAllByText("key it by id")).toHaveLength(1);
  });

  it("Change direction goes through the core's control redirect", async () => {
    const f = open({ stream_window: win(["Working."]), control_redirect: { action_id: "act_8", state: "queued" } });
    const box = await screen.findByRole("textbox", { name: L.ask("Backend") });
    await waitFor(() => expect(box.getAttribute("aria-disabled")).toBeNull());
    fireEvent.change(box, { target: { value: "stop and write tests" } });
    fireEvent.click(screen.getByRole("button", { name: L.redirect }));
    await waitFor(() => expect(f.calls.some((c) => c.cmd === "control_redirect" && c.args?.text === "stop and write tests")).toBe(true));
  });

  it("a refusal keeps the message in the thread with why, and nothing else is tried", async () => {
    open({ stream_window: win(["Working."]), control_message: () => { throw { error: "control_disabled", detail: "no live session" }; } });
    const box = await screen.findByRole("textbox", { name: L.ask("Backend") });
    await waitFor(() => expect(box.getAttribute("aria-disabled")).toBeNull());
    fireEvent.change(box, { target: { value: "hello" } });
    fireEvent.click(screen.getByRole("button", { name: L.send }));
    const log = screen.getByRole("log", { name: L.label });
    await waitFor(() => expect(within(log).getByText(L.delivery.failed)).toBeTruthy());
    expect(within(log).getByText("hello")).toBeTruthy();
  });

  it("a finished worker's box is read only and says why", async () => {
    const done = structuredClone(tree);
    done.children[0].children[0].children[0].status = "done";
    open({ crew: done, stream_window: win(["All done."]) });
    const box = await screen.findByRole("textbox", { name: L.ask("Backend") });
    expect(box.getAttribute("aria-disabled")).toBe("true");
    expect(screen.getByText(L.offFinished)).toBeTruthy();
    expect(screen.getByRole("button", { name: L.send }).getAttribute("aria-disabled")).toBe("true");
  });

  it("a Write step for a file git does not track yet opens to say it is new, not that nothing changed", async () => {
    open({
      stream_window: win(["[tool] Write: /w/test/new.test.ts"]),
      worktree_info: { worker: "a2", worktree: "/w", branch: "b", base: "abc123" },
      diff: { code: 0, diff: "", truncated: false, timed_out: false, untracked: ["test/new.test.ts"] },
    });
    const log = await screen.findByRole("log", { name: L.label });
    const row = await waitFor(() => { const r = log.querySelector("summary"); expect(r).toBeTruthy(); return r as HTMLElement; });
    fireEvent.click(row);
    expect(await within(log).findByText(L.newFile)).toBeTruthy();
    expect(within(log).queryByText(L.noFileDiff)).toBeNull();
  });

  it("a test step opens to its output with the counts on the row, and the row is keyboard reachable", async () => {
    open({ stream_window: win(["[tool] Bash: npm test", "[result] Tests: 3 passed, 3 total"]) });
    const log = await screen.findByRole("log", { name: L.label });
    await waitFor(() => expect(within(log).getByText(L.tests(3, 0))).toBeTruthy());
    const row = log.querySelector("summary") as HTMLElement;
    expect(row.tagName).toBe("SUMMARY");
    expect(row.textContent).toContain("npm test");
  });
});

describe("the Lead's chat", () => {
  it("a message the Lead sent a worker is a card that opens that worker's chat", async () => {
    const f = fakeIpc({
      ...liveRun(), crew: tree, task: { task: {} }, worktree_info: { worker: "a2", worktree: "/w", branch: "b", base: null },
      lead_status: { available: true, busy: false, name: "Lead", stream: "lead-r1", generation: 1 },
      stream_window: (a?: Record<string, unknown>) => (a?.stream === "lead-r1"
        ? win(["you\thow is the backend?", "lead\tI will nudge it.", `tool\tmcp__vajra-desktop__message_worker {"worker":"agent:a2","text":"key it by id"}`])
        : win(["Working on the bucket."])),
    });
    setTransport(f.transport);
    window.location.hash = "#/crew?slug=camp&id=r1";
    render(<Live><Crew /></Live>);
    fireEvent.click(within(await screen.findByRole("tree")).getByText(S.views.orch).closest("[role=treeitem]") as HTMLElement);
    const log = await screen.findByRole("log", { name: S.dg.transcript });
    const card = await within(log).findByRole("button", { name: /Messaged/ });
    expect(card.textContent).toContain("Backend");
    expect(card.textContent).toContain("key it by id");
    fireEvent.click(card);
    await waitFor(() => expect(screen.getByRole("log", { name: L.label }).textContent).toContain("Working on the bucket."));
  });
});
