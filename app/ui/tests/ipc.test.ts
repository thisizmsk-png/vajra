import { afterEach, describe, expect, it } from "vitest";
import { api, errorText, isIpcError, onChanged, setTransport } from "../src/ipc";
import { fakeIpc } from "./fakeIpc";

afterEach(() => setTransport());

describe("typed ipc", () => {
  it("sends camelCase arguments under the command's name", async () => {
    const f = fakeIpc({ tail: { lines: [], gap: false, head_seq: 0, spill_start: null, spill_end: null }, take_over: {} });
    setTransport(f.transport);
    await api.tail("s1", 7);
    await api.takeOver("s1", true);
    expect(f.calls).toEqual([
      { cmd: "tail", args: { stream: "s1", afterSeq: 7, maxBytes: 65536 } },
      { cmd: "take_over", args: { stream: "s1", on: true } },
    ]);
  });

  it("maps the project commands to snake_case commands with camelCase args", async () => {
    const f = fakeIpc({ app_info: {}, project_list: { projects: [], skipped: 0 }, project_add: {}, project_rename: {}, project_archive: {}, project_sessions: { sessions: [] } });
    setTransport(f.transport);
    await api.appInfo();
    await api.projectList();
    await api.projectAdd("/Users/me/src/a");
    await api.projectAdd("/Users/me/src/a/b", true);
    await api.projectRename("p_1", "New", 2);
    await api.projectArchive("p_1");
    await api.projectSessions("p_1");
    expect(f.calls).toEqual([
      { cmd: "app_info", args: undefined },
      { cmd: "project_list", args: { includeArchived: false } },
      { cmd: "project_add", args: { path: "/Users/me/src/a", confirmNested: false } },
      { cmd: "project_add", args: { path: "/Users/me/src/a/b", confirmNested: true } },
      { cmd: "project_rename", args: { id: "p_1", name: "New", expectRev: 2 } },
      { cmd: "project_archive", args: { id: "p_1", expectRev: undefined } },
      { cmd: "project_sessions", args: { id: "p_1" } },
    ]);
  });

  it("maps the migration commands", async () => {
    const f = fakeIpc({ project_migration_suggest: { done: false, groups: [], unsorted: [], terminal: [] }, project_migration_confirm: {} });
    setTransport(f.transport);
    await api.projectMigrationSuggest();
    await api.projectMigrationConfirm(["/Users/me/src/a"]);
    expect(f.calls).toEqual([
      { cmd: "project_migration_suggest", args: undefined },
      { cmd: "project_migration_confirm", args: { roots: ["/Users/me/src/a"] } },
    ]);
  });

  it("maps the three Lead commands to snake_case commands with camelCase args", async () => {
    const f = fakeIpc({ ask_lead: {}, cancel_ask: {}, lead_status: { available: true, busy: false } });
    setTransport(f.transport);
    await api.askLead("20261106-0911", "what is bhima doing?");
    await api.cancelAsk("01ARZ3NDEKTSV4RRFFQ69G5FAV");
    await api.leadStatus("20261106-0911");
    expect(f.calls).toEqual([
      { cmd: "ask_lead", args: { runId: "20261106-0911", text: "what is bhima doing?" } },
      { cmd: "cancel_ask", args: { askId: "01ARZ3NDEKTSV4RRFFQ69G5FAV" } },
      { cmd: "lead_status", args: { runId: "20261106-0911" } },
    ]);
  });

  it("maps approve and reject to their commands with camelCase arguments", async () => {
    const f = fakeIpc({ approve_plan: { action_id: "a1", state: "delivered", via: "session" }, reject_plan: { action_id: "a2", state: "rejected" } });
    setTransport(f.transport);
    await api.approvePlan("plan:20261106-0911.0", "ab".repeat(32));
    await api.rejectPlan("plan:20261106-0911.0", "too risky");
    expect(f.calls).toEqual([
      { cmd: "approve_plan", args: { id: "plan:20261106-0911.0", planSha256: "ab".repeat(32) } },
      { cmd: "reject_plan", args: { id: "plan:20261106-0911.0", reason: "too risky" } },
    ]);
  });

  it("maps inbox to the inbox command", async () => {
    const f = fakeIpc({ inbox: { count: 0, items: [] } });
    setTransport(f.transport);
    expect(await api.inbox()).toEqual({ count: 0, items: [] });
    expect(f.calls).toEqual([{ cmd: "inbox", args: undefined }]);
  });

  it("shows the sandbox message as one friendly line", () => {
    const msg = "Ask the Lead needs the Claude Code sandbox. Run /sandbox in Claude Code, then retry.";
    expect(errorText({ error: "lead_guard_missing", detail: msg })).toBe(msg);
    expect(errorText({ error: "lead_busy" })).toBe("lead_busy");
  });

  it("rejects with the core's {error, detail} and errorText reads it", async () => {
    setTransport(fakeIpc({ preflight: () => { throw { error: "no_claude", detail: "not on PATH" }; } }).transport);
    const e = await api.preflight().catch((x) => x);
    expect(isIpcError(e)).toBe(true);
    expect(errorText(e)).toBe("no_claude: not on PATH");
  });

  it("never shows a stack or an unknown object", () => {
    expect(errorText(new Error("boom\n at x"))).toBe("unexpected error");
    expect(errorText(null)).toBe("unexpected error");
    expect(errorText({ error: "gone" })).toBe("gone");
  });

  it("onChanged subscribes to the changed event and unsubscribes", () => {
    const f = fakeIpc();
    setTransport(f.transport);
    const seen: unknown[] = [];
    const off = onChanged((p) => seen.push(p));
    f.emit("changed", ["a"]);
    off();
    f.emit("changed", ["b"]);
    expect(seen).toEqual([["a"]]);
    expect(f.listenerCount("changed")).toBe(0);
  });
});
