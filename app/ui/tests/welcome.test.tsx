// The first-run welcome (option A): one test per state in the design, the reusable row, and the keyboard path.
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { setTransport } from "../src/ipc";
import { forgetRead } from "../src/reads";
import { Shell } from "../src/Shell";
import { CheckList, CheckRow } from "../src/welcome/CheckRow";
import { derive } from "../src/welcome/model";
import type { AgentInfo } from "../src/welcome/model";
import { toolsOf } from "../src/welcome/Welcome";
import { S } from "../strings";
import { fakeIpc } from "./fakeIpc";

const W = S.welcome;
const P = "p_01JC5X8Z3A9K2M4N6P7Q8R9S0T";
const project = { schema: "vajra.project/1", id: P, name: "vajra-site", root: "/Users/me/code/vajra-site", created: null, archived: null, rev: 1 };
const REPOS = [
  { root: "/Users/me/code/vajra-site", name: "vajra-site", worktrees: 0 },
  { root: "/Users/me/code/billing-api", name: "billing-api", worktrees: 2 },
  { root: "/Users/me/dotfiles", name: "dotfiles", worktrees: 0 },
  { root: "/Users/me/Developer/notes-app", name: "notes-app", worktrees: 1 },
];
const GIT_OK = [{ name: "git", ok: true, detail: "2.43", fix: "" }, { name: "python3", ok: true, detail: "3.12", fix: "" }];
const GIT_MISSING = [{ name: "git", ok: false, detail: "not found", fix: "", action: "install_dev_tools" as const, action_label: "Install", action_note: "" }, { name: "python3", ok: false, detail: "not found", fix: "" }];

const providers = (state: "signed_in" | "signed_out" | "not_installed") => {
  const base = JSON.parse(readFileSync(resolve(__dirname, "../../core/tests/contract/providers.v1.json"), "utf8"));
  base.providers = base.providers.filter((p: { id: string }) => p.id === "claude").map((p: { login: object; installed: boolean }) => ({ ...p, installed: state !== "not_installed", login: { ...p.login, state } }));
  return base;
};
const setupRow = (id: string, name: string, state: string, version: string | null, extra: object = {}) =>
  ({ id, name, state, version, path: state === "installed" ? `/usr/local/bin/${id}` : null, reason_code: state === "installed" ? null : "not_installed", commands: [], docs_url: "https://example.com/docs", can_sign_in: id === "claude" && state === "installed", one_click: false, ...extra });
const setup = (claude: "installed" | "manual", engine: boolean) => ({
  schema: "vajra.harness_setup/1",
  rows: [setupRow("claude", "Claude Code", claude, claude === "installed" ? "2.1.323" : null), setupRow("codex", "Codex CLI", "manual", null), setupRow("gemini", "Gemini CLI", "manual", null)],
  engine: { installed: engine, command: "bash ~/.claude/skills/vajra/scripts/install.sh" },
});

interface World { discover: unknown; pre: unknown; claude: "installed" | "manual"; login: "signed_in" | "signed_out" | "not_installed"; engine: boolean; listed?: boolean; extra?: Record<string, unknown> }
let f: ReturnType<typeof fakeIpc>;
function boot(w: Partial<World> = {}, hash = "") {
  const world: World = { discover: { found: REPOS, truncated: false }, pre: { rows: GIT_MISSING, start_enabled: false }, claude: "manual", login: "not_installed", engine: false, ...w };
  let added = world.listed ?? false;
  window.location.hash = hash;
  f = fakeIpc({
    project_list: () => ({ projects: added ? [project] : [], skipped: 0, truncated: false }),
    project_discover: () => { if (world.discover instanceof Error) throw world.discover; return world.discover; },
    project_add: () => { added = true; return { outcome: "created", project }; },
    preflight: world.pre, provider_list: () => providers(world.login), setup_list: () => setup(world.claude, world.engine),
    list_campaigns: { campaigns: [] }, resume_targets: { targets: {} }, active_runs: { count: 0, runs: [] }, inbox: { count: 0, items: [] },
    act_start_run: { receipt: { state: "done" }, result: { campaign: "fix-date", run_id: "r1", stream: "s1" } },
    ...world.extra,
  });
  setTransport(f.transport);
  return render(<Shell />);
}
const rowHead = (name: string) => screen.getByRole("button", { name: new RegExp(`^\\d?\\s*${name}`) });
const startButton = () => screen.getByRole("button", { name: new RegExp(`^${W.start}`) });

/** An in-memory Storage, fresh per test: a project remembered by one test must not reach the next (the first launch has none). */
function memoryStorage(): Storage {
  const m = new Map<string, string>();
  return {
    get length() { return m.size; }, clear: () => m.clear(), getItem: (k) => m.get(k) ?? null, key: (i) => [...m.keys()][i] ?? null,
    removeItem: (k) => { m.delete(k); }, setItem: (k, v) => { m.set(k, String(v)); },
  };
}
beforeEach(() => {
  HTMLDialogElement.prototype.showModal = function (this: HTMLDialogElement) { this.setAttribute("open", ""); };
  HTMLDialogElement.prototype.close = function (this: HTMLDialogElement) { this.removeAttribute("open"); };
  Object.defineProperty(window, "localStorage", { configurable: true, value: memoryStorage() });
  Object.defineProperty(window, "innerWidth", { configurable: true, value: 1440 });
  forgetRead();
});
afterEach(() => { cleanup(); setTransport(); window.location.hash = ""; });

describe("welcome: the seven states in the design", () => {
  it("1 clean Mac, repos found: the folder row opens by itself and focus lands on the first repository", async () => {
    boot();
    await screen.findByRole("heading", { name: W.title });
    expect(await screen.findByRole("radiogroup", { name: W.folder.repos })).toBeTruthy();
    expect(screen.getAllByRole("radio")).toHaveLength(4);
    expect(rowHead(W.folder.title).getAttribute("aria-expanded")).toBe("true");
    expect(rowHead(W.agent.title).getAttribute("aria-expanded")).toBe("false");
    expect(within(rowHead(W.folder.title)).getByText(W.folder.found(4))).toBeTruthy();
    await waitFor(() => expect(document.activeElement).toBe(screen.getAllByRole("radio")[0]));
    // Nothing is written until a click, and Start says why it is off.
    expect(f.calls.some((c) => c.cmd === "project_add")).toBe(false);
    expect(startButton().getAttribute("aria-disabled")).toBe("true");
    expect(screen.getByText(W.finish(3))).toBeTruthy();
    expect(screen.getByText(W.progress(0))).toBeTruthy();
    expect(screen.getByText(S.welcome.windowTitle)).toBeTruthy();
  });

  it("2 no repos found: says where it looked and offers the system folder picker; a pasted path works too", async () => {
    boot({ discover: { found: [], truncated: false } });
    expect(await screen.findByText(W.folder.noneBody)).toBeTruthy();
    expect(within(rowHead(W.folder.title)).getByText(W.folder.none)).toBeTruthy();
    expect(screen.getByRole("button", { name: W.folder.choose })).toBeTruthy();
    fireEvent.change(screen.getByLabelText(W.folder.pathLabel), { target: { value: "/Users/me/new-app" } });
    fireEvent.click(screen.getByRole("button", { name: W.folder.usePath }));
    await waitFor(() => expect(f.calls.find((c) => c.cmd === "project_add")?.args).toMatchObject({ path: "/Users/me/new-app" }));
  });

  it("3 scan failed: says what failed in words, and the folder picker still works", async () => {
    boot({ extra: { project_discover: () => { throw { error: "io_error", detail: "the folder list could not be read" }; } } });
    expect(await screen.findByText(W.folder.failedTitle)).toBeTruthy();
    expect(screen.getByText(/the folder list could not be read/)).toBeTruthy();
    expect(document.body.textContent).not.toContain("io_error"); // the core's words, never its code
    expect(within(rowHead(W.folder.title)).getByText(W.folder.failed)).toBeTruthy();
    expect(screen.getByRole("button", { name: W.folder.choose })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: W.folder.again }));
    await waitFor(() => expect(f.calls.filter((c) => c.cmd === "project_discover")).toHaveLength(2));
  });

  it("4 agent signed out: folder and tools show Ready, the agent row opens on its own, Sign in opens the CLI's own login tab", async () => {
    boot({ pre: { rows: GIT_OK, start_enabled: true }, claude: "installed", login: "signed_out" });
    fireEvent.click((await screen.findAllByRole("radio"))[0]);
    await waitFor(() => expect(rowHead(W.agent.title).getAttribute("aria-expanded")).toBe("true"));
    expect(f.calls.find((c) => c.cmd === "project_add")?.args).toMatchObject({ path: REPOS[0].root });
    expect(within(rowHead(W.folder.title)).getByText(W.word.pass)).toBeTruthy();
    expect(within(rowHead(W.agent.title)).getByText(W.agent.signedOutSummary("Claude Code"))).toBeTruthy();
    let asked: unknown = null;
    window.addEventListener("vajra:setup-sign-in", (e) => { asked = (e as CustomEvent).detail; }, { once: true });
    fireEvent.click(await screen.findByRole("button", { name: W.agent.signIn("Claude Code") }));
    expect(asked).toMatchObject({ id: "claude" });
    expect(screen.getByText(W.finish(2))).toBeTruthy();
  });

  it("5 engine missing: the one Terminal step in plain words, the lines to paste and a Copy button", async () => {
    boot({ pre: { rows: GIT_OK, start_enabled: true }, claude: "installed", login: "signed_in" });
    fireEvent.click((await screen.findAllByRole("radio"))[0]);
    await waitFor(() => expect(rowHead(W.engine.title).getAttribute("aria-expanded")).toBe("true"));
    expect(screen.getByText(W.engine.intro)).toBeTruthy();
    expect(screen.getByLabelText(W.engine.lines).textContent).toContain("bash ~/.claude/skills/vajra/scripts/install.sh");
    expect(screen.getByRole("button", { name: W.engine.copy })).toBeTruthy();
    expect(within(rowHead(W.agent.title)).getByText(W.agent.readySummary("Claude Code", "2.1.323"))).toBeTruthy();
    expect(screen.getByText(W.finish(1))).toBeTruthy();
    expect(startButton().getAttribute("aria-disabled")).toBe("true");
  });

  it("6 ready: the three rows collapse, the ask gets focus, Start run sends the same call Start a run sends", async () => {
    boot({ pre: { rows: GIT_OK, start_enabled: true }, claude: "installed", login: "signed_in", engine: true });
    fireEvent.click((await screen.findAllByRole("radio"))[0]);
    const ask = await screen.findByLabelText(W.ask.label);
    await waitFor(() => expect(document.activeElement).toBe(ask));
    expect(screen.getByText(W.ready)).toBeTruthy();
    for (const t of [W.folder.title, W.agent.title, W.engine.title]) expect(rowHead(t).getAttribute("aria-expanded")).toBe("false");
    expect(startButton().getAttribute("aria-disabled")).toBeNull();
    // An empty ask is refused with a reason, not sent.
    fireEvent.click(startButton());
    expect(await screen.findByText(W.ask.empty)).toBeTruthy();
    expect(f.calls.some((c) => c.cmd === "act_start_run")).toBe(false);
    fireEvent.change(ask, { target: { value: "Fix the date header on the home page" } });
    fireEvent.keyDown(ask, { key: "Enter", metaKey: true });
    await waitFor(() => expect(f.calls.filter((c) => c.cmd === "act_start_run")).toHaveLength(1));
    const call = f.calls.find((c) => c.cmd === "act_start_run")!.args as { envelope: { project_id: string }; base: string; ask: string; maxRounds: number; cwd: string };
    expect([call.envelope.project_id, call.base, call.ask, call.maxRounds, call.cwd]).toEqual([P, "HEAD", "Fix the date header on the home page", 3, project.root]);
    await waitFor(() => expect(window.location.hash).toContain("/review?"));
    expect(window.location.hash).toContain("first=1");
  });

  it("7 first run done: the Review view carries a one-time line about what the rows mean", async () => {
    boot({ listed: true }, `#/p/${P}/review?slug=fix-date&first=1`);
    expect(await screen.findByText(W.done.note)).toBeTruthy();
    expect(screen.getByRole("button", { name: W.done.another })).toBeTruthy();
    cleanup();
    boot({ listed: true }, `#/p/${P}/review?slug=fix-date`);
    await screen.findByRole("heading", { name: S.run.title });
    expect(screen.queryByText(W.done.note)).toBeNull();
  });
});

describe("welcome: behaviour that has no drawing", () => {
  it("never starts a poll: at rest it makes no call after the first reads settle", async () => {
    boot();
    await screen.findAllByRole("radio");
    await act(async () => { await new Promise((r) => setTimeout(r, 50)); });
    const n = f.calls.length;
    await act(async () => { await new Promise((r) => setTimeout(r, 300)); });
    expect(f.calls.length).toBe(n);
  });

  it("checks again when you come back to the window, not on a timer", async () => {
    boot({ pre: { rows: GIT_OK, start_enabled: true }, claude: "manual" });
    await screen.findAllByRole("radio");
    await act(async () => { await new Promise((r) => setTimeout(r, 50)); });
    const before = f.calls.filter((c) => c.cmd === "setup_list").length;
    act(() => { window.dispatchEvent(new Event("focus")); });
    await waitFor(() => expect(f.calls.filter((c) => c.cmd === "setup_list").length).toBe(before + 1));
  });

  it("a folder that cannot be added says so and leaves the row open", async () => {
    boot({ extra: { project_add: () => { throw { error: "refused_folder", detail: "that folder is not allowed" }; } } });
    fireEvent.click((await screen.findAllByRole("radio"))[1]);
    expect((await screen.findByText(new RegExp(W.fail.add))).textContent).toContain(W.fail.add);
    expect(rowHead(W.folder.title).getAttribute("aria-expanded")).toBe("true");
  });

  it("Settings still opens over the welcome", async () => {
    boot({}, "#/settings");
    await waitFor(() => expect(document.title).toContain(S.views.settings));
    expect(screen.queryByRole("heading", { name: W.title })).toBeNull();
  });

  it("ticking 'also add the others' adds them too; unticked adds only the pick", async () => {
    boot();
    fireEvent.click(await screen.findByRole("checkbox"));
    fireEvent.click(screen.getAllByRole("radio")[0]);
    await waitFor(() => expect(f.calls.filter((c) => c.cmd === "project_add")).toHaveLength(1));
  });
});

describe("welcome: the row is reusable and keyboard-first", () => {
  const rows = (open = 0) => (
    <CheckList label="Checks">
      {["One", "Two", "Three"].map((t, i) => (
        <CheckRow key={t} n={i + 1} title={t} kind={i === 0 ? "fail" : "todo"} summary={`${t} summary`} open={open === i} onToggle={() => {}}>
          <button type="button">{t} action</button>
        </CheckRow>
      ))}
    </CheckList>
  );
  it("a failed check is the same row: shape and word, never colour alone", () => {
    render(rows());
    const head = screen.getAllByRole("button", { name: /^One/ })[0];
    expect(within(head).getByText(W.word.fail)).toBeTruthy();
    expect(head.querySelector("svg")).toBeTruthy();
    expect(head.getAttribute("aria-expanded")).toBe("true");
    expect(screen.getByRole("region", { name: /^One/ })).toBeTruthy();
  });
  it("up and down move between row heads; Home and End jump; nothing else moves", () => {
    render(rows());
    const heads = screen.getAllByRole("button", { name: /summary/ });
    heads[0].focus();
    fireEvent.keyDown(heads[0], { key: "ArrowDown" });
    expect(document.activeElement).toBe(heads[1]);
    fireEvent.keyDown(heads[1], { key: "End" });
    expect(document.activeElement).toBe(heads[2]);
    fireEvent.keyDown(heads[2], { key: "ArrowUp" });
    expect(document.activeElement).toBe(heads[1]);
    fireEvent.keyDown(heads[1], { key: "Home" });
    expect(document.activeElement).toBe(heads[0]);
    fireEvent.keyDown(heads[0], { key: "ArrowUp" });
    expect(document.activeElement).toBe(heads[0]);
  });
});

describe("welcome: what each check means (pure)", () => {
  const agent = (state: string, login: "signed_in" | "signed_out" | "cannot_tell" | "not_installed"): AgentInfo =>
    ({ row: { id: "claude", name: "Claude Code", state: state as "installed", version: "2.1", path: null, reason_code: null, commands: [], docs_url: "https://x.y", can_sign_in: true, one_click: false }, login });
  const base = { repos: { at: "found" as const, list: REPOS, truncated: false }, picked: null, tools: "ok" as const, agents: [agent("installed", "signed_in")], engine: true };
  it("counts a check ready only on a clear yes; 'cannot tell' and a failed read never count", () => {
    expect(derive({ ...base, picked: { id: P, name: "vajra-site", root: "/r" } }).ready).toBe(3);
    expect(derive({ ...base, picked: { id: P, name: "vajra-site", root: "/r" }, agents: [agent("installed", "cannot_tell")] }).agent.kind).toBe("todo");
    expect(derive({ ...base, agents: "failed" }).agent.summary).toBe(W.agent.failedSummary);
    expect(derive({ ...base, engine: "failed" }).engine.kind).toBe("todo");
  });
  it("the first row that needs you opens: folder, then agent, then engine, then the ask", () => {
    const pk = { id: P, name: "n", root: "/r" };
    expect(derive(base).first).toBe(1);
    expect(derive({ ...base, picked: pk, agents: [agent("installed", "signed_out")] }).first).toBe(2);
    expect(derive({ ...base, picked: pk, tools: "missing" }).first).toBe(2);
    expect(derive({ ...base, picked: pk, engine: false }).first).toBe(3);
    expect(derive({ ...base, picked: pk }).first).toBe(4);
  });
  it("developer tools are missing only when git or Python 3 is reported failing", () => {
    expect(toolsOf(null)).toBe("unknown");
    expect(toolsOf({ rows: [], start_enabled: true })).toBe("ok");
    expect(toolsOf({ rows: GIT_OK, start_enabled: true })).toBe("ok");
    expect(toolsOf({ rows: GIT_MISSING, start_enabled: false })).toBe("missing");
  });
});
