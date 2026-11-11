// ADR-17 step 1 in the UI: Settings > Providers > Set up. Each harness's state, the vendor's own steps for one that is
// missing or broken (never an Install button in this step), and the CLI's own sign-in in a terminal tab.
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { setTransport } from "../src/ipc";
import { parseSetupList, SETUP_SCHEMA } from "../src/setupContract";
import { SetupPanel } from "../src/screens/settings/SetupPanel";

const SIGN_IN_EVENT = "vajra:setup-sign-in";
import { S } from "../strings";
import { fakeIpc } from "./fakeIpc";

const C = S.setup;

const cmds = (name: string) => [{ tool: "brew", command: `brew install --cask ${name}` }, { tool: "npm", command: `npm install -g @v/${name}` }];
const LIST = {
  schema: SETUP_SCHEMA,
  rows: [
    { id: "claude", name: "Claude Code", state: "installed", version: "2.1.322", path: "/opt/homebrew/bin/claude", reason_code: null, commands: [], docs_url: "https://code.claude.com/docs/en/setup", can_sign_in: true },
    { id: "codex", name: "Codex CLI", state: "broken", version: null, path: "/opt/homebrew/bin/codex", reason_code: "version_failed", commands: cmds("codex"), docs_url: "https://github.com/openai/codex", can_sign_in: false },
    { id: "gemini", name: "Gemini CLI", state: "unsupported", version: "0.68.0", path: "/opt/homebrew/bin/gemini", reason_code: null, commands: [], docs_url: "https://github.com/google-gemini/gemini-cli", can_sign_in: true },
    { id: "other", name: "Other CLI", state: "manual", version: null, path: null, reason_code: "source_not_verified", commands: cmds("other"), docs_url: "https://example.com/other", can_sign_in: false },
  ],
  engine: { installed: false, command: "bash ~/.claude/skills/vajra/scripts/install.sh" },
};

afterEach(() => {
  cleanup();
  setTransport();
});

describe("parseSetupList", () => {
  it("reads a valid answer", () => {
    const r = parseSetupList(LIST);
    expect(r.ok && r.value.rows.map((x) => x.state)).toEqual(["installed", "broken", "unsupported", "manual"]);
  });
  it("reads another major as an update", () => {
    const r = parseSetupList({ ...LIST, schema: "vajra.harness_setup/2" });
    expect(!r.ok && r.reason).toBe("unsupported_version");
  });
  it("reads an unknown state as install it yourself, never as installed", () => {
    const r = parseSetupList({ ...LIST, rows: [{ ...LIST.rows[0], state: "magic" }] });
    expect(r.ok && r.value.rows[0].state).toBe("manual");
    expect(r.ok && r.value.rows[0].can_sign_in).toBe(false);
  });
  it("refuses a docs link that is not https and a row without an id", () => {
    expect(parseSetupList({ ...LIST, rows: [{ ...LIST.rows[3], docs_url: "javascript:alert(1)" }] }).ok).toBe(false);
    expect(parseSetupList({ ...LIST, rows: [{ ...LIST.rows[3], id: "" }] }).ok).toBe(false);
  });
  it("keeps only the named fields", () => {
    const r = parseSetupList({ ...LIST, rows: [{ ...LIST.rows[0], token: "secret" }] });
    expect(JSON.stringify(r)).not.toContain("secret");
  });
});

function panel(list: unknown = LIST) {
  const ipc = fakeIpc({ setup_list: list });
  setTransport(ipc.transport);
  render(<SetupPanel />);
  return ipc;
}

const rowOf = async (name: string) => within(await screen.findByRole("region", { name }));

describe("SetupPanel", () => {
  it("shows an installed harness with its version and a Sign in button", async () => {
    panel();
    const r = await rowOf("Claude Code");
    expect(r.getByText("2.1.322")).toBeTruthy();
    expect(r.getByText(C.state.installed)).toBeTruthy();
    expect(r.getByRole("button", { name: C.signIn("Claude Code") })).toBeTruthy();
  });

  it("says a set-up harness without an adapter can't run as an agent yet", async () => {
    panel();
    const r = await rowOf("Gemini CLI");
    expect(r.getByText(C.state.unsupported)).toBeTruthy();
    expect(r.getByRole("button", { name: C.signIn("Gemini CLI") })).toBeTruthy();
  });

  it("shows a broken harness with the vendor's own repair steps and no Sign in", async () => {
    panel();
    const r = await rowOf("Codex CLI");
    expect(r.getByText(C.state.broken)).toBeTruthy();
    expect(r.getByText(C.reason.version_failed)).toBeTruthy();
    expect(r.getByText("brew install --cask codex")).toBeTruthy();
    expect(r.getByLabelText(C.vendorSteps("Codex CLI")).textContent).toBe("https://github.com/openai/codex");
    expect(r.queryByRole("button", { name: C.signIn("Codex CLI") })).toBeNull();
  });

  it("offers a missing harness as install it yourself, with no Install button", async () => {
    panel();
    const r = await rowOf("Other CLI");
    expect(r.getByText(C.state.manual)).toBeTruthy();
    expect(r.getByText("npm install -g @v/other")).toBeTruthy();
    expect(r.getByLabelText(C.vendorSteps("Other CLI")).textContent).toBe("https://example.com/other");
    expect(screen.queryAllByRole("link")).toEqual([]);
    expect(screen.queryByRole("button", { name: /install/i })).toBeNull();
  });

  it("shows the engine scripts row as detection only", async () => {
    panel();
    const r = await rowOf(C.engine);
    expect(r.getByText(C.engineState.missing)).toBeTruthy();
    expect(r.getByText("bash ~/.claude/skills/vajra/scripts/install.sh")).toBeTruthy();
  });

  it("asks the terminal dock for a sign-in tab by harness id only", async () => {
    panel();
    const r = await rowOf("Claude Code");
    const seen: unknown[] = [];
    const on = (e: Event) => seen.push((e as CustomEvent).detail);
    window.addEventListener(SIGN_IN_EVENT, on);
    fireEvent.click(r.getByRole("button", { name: C.signIn("Claude Code") }));
    window.removeEventListener(SIGN_IN_EVENT, on);
    expect(seen).toEqual([{ id: "claude", label: C.signInTab("Claude Code") }]);
  });

  it("says when the core answers something it cannot read", async () => {
    panel({ schema: "vajra.harness_setup/9", rows: [] });
    await waitFor(() => expect(screen.getByText(C.updateNeeded)).toBeTruthy());
  });
});

// ADR-17 17b: one click, only for a verified source. The sheet shows the exact command and everything it binds; Install
// asks for a consent token for that plan's hash and runs with it; the token never reaches the page.
const HASH = "c".repeat(64);
const PLAN = {
  schema: "vajra.setup_plan/1",
  id: "other",
  plan: {
    id: "other",
    method: "npm",
    command: ["/opt/homebrew/bin/node", "/opt/homebrew/lib/node_modules/npm/bin/npm-cli.js", "install", "-g", "--ignore-scripts", "--registry=https://registry.npmjs.org/", "<verified tarball>"],
    tools: [
      { role: "node", path: "/opt/homebrew/bin/node", label: null, owned_by_you: true },
      { role: "npm", path: "/opt/homebrew/lib/node_modules/npm/bin/npm-cli.js", label: null, owned_by_you: true },
    ],
    source: "registry.npmjs.org",
    version: "1.2.3",
    target: "/opt/homebrew",
    pin: { kind: "npm", tgz_sha512: "sha512-x" },
    needs_admin: null,
    notes: ["npmrc_applies", "owned_by_you"],
    scope_registry: "https://mirror.example/",
    deps: [],
    hash: HASH,
  },
};
const TOKEN = "f".repeat(32);
const oneClick = { ...LIST, rows: [{ ...LIST.rows[3], one_click: true, reason_code: "not_installed" }, { ...LIST.rows[1], one_click: true }] };

function sheet(plan: unknown = PLAN, run: unknown = { stream: "setup-other", report: { outcome: "installed", reason: null, exit: 0, version: "1.2.3" } }) {
  const ipc = fakeIpc({
    setup_list: oneClick,
    setup_plan: plan,
    setup_consent: { token: TOKEN },
    setup_run: run,
    tail: { lines: [{ seq: 1, text: "added 1 package" }], gap: false, head_seq: 1, spill_start: null, spill_end: null },
  });
  setTransport(ipc.transport);
  render(<SetupPanel />);
  return ipc;
}

describe("one-click install", () => {
  it("is not offered for an unverified source", async () => {
    panel();
    await rowOf("Other CLI");
    expect(screen.queryByRole("button", { name: C.planOneClick("Other CLI") })).toBeNull();
  });

  it("shows the exact command and what it binds before anything installs", async () => {
    const ipc = sheet();
    const r = await rowOf("Other CLI");
    fireEvent.click(r.getByRole("button", { name: C.planOneClick("Other CLI") }));
    const s = within(await screen.findByRole("region", { name: C.sheetTitle("Other CLI") }));
    expect(s.getByText(PLAN.plan.command.join(" "))).toBeTruthy();
    expect(s.getByText("registry.npmjs.org")).toBeTruthy();
    expect(s.getByText("1.2.3")).toBeTruthy();
    expect(s.getByText("/opt/homebrew")).toBeTruthy();
    expect(s.getAllByText(C.ownedByYou).length).toBe(2);
    expect(s.getByText("https://mirror.example/")).toBeTruthy();
    expect(s.getByText(C.note.npmrc_applies)).toBeTruthy();
    expect(ipc.calls.filter((c) => c.cmd === "setup_plan")).toEqual([{ cmd: "setup_plan", args: { id: "other" } }]);
    expect(ipc.calls.some((c) => c.cmd === "setup_consent" || c.cmd === "setup_run")).toBe(false);
  });

  it("asks for consent on the plan's hash, runs with the token, shows the live output and the result", async () => {
    const ipc = sheet();
    fireEvent.click((await rowOf("Other CLI")).getByRole("button", { name: C.planOneClick("Other CLI") }));
    const s = within(await screen.findByRole("region", { name: C.sheetTitle("Other CLI") }));
    fireEvent.click(s.getByRole("button", { name: C.install("Other CLI") }));
    await waitFor(() => expect(s.getByText(C.outcome.installed)).toBeTruthy());
    const cmds = ipc.calls.map((c) => c.cmd).filter((c) => c.startsWith("setup_") && c !== "setup_list");
    expect(cmds).toEqual(["setup_plan", "setup_consent", "setup_run"]);
    expect(ipc.calls.find((c) => c.cmd === "setup_consent")!.args).toEqual({ id: "other", hash: HASH });
    expect(ipc.calls.find((c) => c.cmd === "setup_run")!.args).toEqual({ id: "other", token: TOKEN });
    expect(document.body.innerHTML).not.toContain(TOKEN);
    expect(ipc.calls.filter((c) => c.cmd === "setup_list").length).toBeGreaterThan(1);
  });

  it("names a folder that needs admin and offers no Install", async () => {
    sheet({ ...PLAN, plan: { ...PLAN.plan, needs_admin: { path: "/usr/local/lib/node_modules", reason: "root_owned" } } });
    fireEvent.click((await rowOf("Other CLI")).getByRole("button", { name: C.planOneClick("Other CLI") }));
    const s = within(await screen.findByRole("region", { name: C.sheetTitle("Other CLI") }));
    expect(s.getByText(C.admin.root_owned)).toBeTruthy();
    expect(s.getByText("/usr/local/lib/node_modules")).toBeTruthy();
    expect(s.queryByRole("button", { name: C.install("Other CLI") })).toBeNull();
  });

  it("says why when the core answers install it yourself", async () => {
    sheet({ schema: "vajra.setup_plan/1", id: "other", manual: "unpinned_dependency" });
    fireEvent.click((await rowOf("Other CLI")).getByRole("button", { name: C.planOneClick("Other CLI") }));
    await waitFor(() => expect(screen.getByText(C.manual.unpinned_dependency)).toBeTruthy());
    expect(screen.queryByRole("button", { name: C.install("Other CLI") })).toBeNull();
  });

  it("offers Repair for a broken harness and shows a rollback plainly", async () => {
    sheet({ ...PLAN, id: "codex", plan: { ...PLAN.plan, id: "codex" } }, { stream: "setup-codex", report: { outcome: "rolled_back", reason: "closure_mismatch", exit: 0, version: null } });
    fireEvent.click((await rowOf("Codex CLI")).getByRole("button", { name: C.planOneClick("Codex CLI") }));
    const s = within(await screen.findByRole("region", { name: C.sheetTitle("Codex CLI") }));
    fireEvent.click(s.getByRole("button", { name: C.repair("Codex CLI") }));
    await waitFor(() => expect(s.getByText(C.outcome.rolled_back)).toBeTruthy());
  });
});

describe("parseSetupPlan", () => {
  it("refuses a plan without a 64-hex hash or with a non-text command", async () => {
    const { parseSetupPlan } = await import("../src/setupContract");
    expect(parseSetupPlan(PLAN).ok).toBe(true);
    expect(parseSetupPlan({ ...PLAN, plan: { ...PLAN.plan, hash: "x" } }).ok).toBe(false);
    expect(parseSetupPlan({ ...PLAN, plan: { ...PLAN.plan, command: [1] } }).ok).toBe(false);
    expect(parseSetupPlan({ ...PLAN, schema: "vajra.setup_plan/2" }).ok).toBe(false);
  });
});
