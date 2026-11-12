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
