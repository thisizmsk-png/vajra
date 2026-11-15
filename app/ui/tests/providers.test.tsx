// FG-09 in the UI: the provider choice in Settings > Providers, what each provider can do (every control that is off is
// shown disabled with its reason, never hidden), Start passing the chosen provider, and the run's provider in Crew/Run.
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { setTransport } from "../src/ipc";
import { PreferencesProvider } from "../src/preferences";
import { DEFAULT_PREFERENCES } from "../src/preferenceContract";
import { CurrentProject } from "../src/projects/current";
import { Settings } from "../src/screens/settings/Settings";
import { Home } from "../src/screens/home/Home";
import { Run } from "../src/screens/run/Run";
import { Crew } from "../src/screens/crew/Crew";
import { S } from "../strings";
import { fakeIpc } from "./fakeIpc";

const fx = (name: string) => JSON.parse(readFileSync(resolve(__dirname, "../../core/tests/contract", name), "utf8"));
const LIST = fx("providers.v1.json");
const P = "p_01JC5X8Z3A9K2M4N6P7Q8R9S0T";
const C = S.providers;
const SETUP_EMPTY = { schema: "vajra.harness_setup/1", rows: [], engine: { installed: true, command: "engine setup command" } };

const view = (project: string | null = null, agent: "claude" | "codex" = "claude", rev = 0) => ({
  schema: "vajra.preferences/1", rev, project_id: project, caps: { active_runs: 1, workers_per_run: 16, editable: false },
  global: { ...DEFAULT_PREFERENCES }, overrides: agent === "claude" ? {} : { agent },
  effective: { ...DEFAULT_PREFERENCES, agent },
  sources: Object.fromEntries(Object.keys(DEFAULT_PREFERENCES).map((k) => [k, k === "agent" && agent !== "claude" ? "project" : "global"])),
});
const codexMissing = () => ({ ...LIST, providers: [LIST.providers[0], { ...LIST.providers[1], installed: false, path: null, version: null, login: { ...LIST.providers[1].login, state: "not_installed" } }] });
const project = { schema: "vajra.project/1", id: P, name: "harbor-api", root: "/src/api", created: null, archived: null, rev: 0 };
const preflight = { rows: [{ name: "claude", ok: true, detail: "2.1.0", fix: "" }], start_enabled: true };

afterEach(() => {
  cleanup();
  setTransport();
  window.location.hash = "";
});

function settings(replies: Record<string, unknown>, inProject = true) {
  window.location.hash = "#/settings";
  const f = fakeIpc({ preflight, provider_list: LIST, setup_list: SETUP_EMPTY, preferences_read: (a?: Record<string, unknown>) => view(a?.projectId as string | null), ...replies });
  setTransport(f.transport);
  render(
    <CurrentProject.Provider value={{ project: inProject ? project : null, campaigns: [], focus: null, runId: null }}>
      <PreferencesProvider projectId={inProject ? P : null}><Settings /></PreferencesProvider>
    </CurrentProject.Provider>,
  );
  return f;
}

describe("Settings > Providers (FG-09)", () => {
  it("shows each provider: version with its tested words, login in plain words, and a key NAME only", async () => {
    settings({});
    const codex = await screen.findByRole("region", { name: "Codex CLI" });
    expect(within(codex).getByText("0.9.0")).toBeTruthy();
    expect(within(codex).getByText(C.tested.untested)).toBeTruthy();
    expect(within(codex).getByText(C.loginState.cannot_tell)).toBeTruthy();
    expect(within(codex).getByText(C.detailCode.check_not_understood)).toBeTruthy();
    expect(within(codex).getByText("Uses your Codex login")).toBeTruthy();
    expect(within(codex).getByText(C.keyInEnv("OPENAI_API_KEY", "Codex CLI"))).toBeTruthy();
    const claude = screen.getByRole("region", { name: "Claude Code" });
    expect(within(claude).getByText(C.loginState.signed_in)).toBeTruthy();
    expect(within(claude).getByText(C.tested.tested)).toBeTruthy();
  });

  it("shows every control that is off as a disabled control with its reason, never hidden", async () => {
    settings({});
    const codex = await screen.findByRole("region", { name: "Codex CLI" });
    const message = within(codex).getByRole("button", { name: C.capability.message });
    expect(message.getAttribute("aria-disabled")).toBe("true");
    const reason = document.getElementById(message.getAttribute("aria-describedby") ?? "");
    expect(reason?.textContent).toBe(C.limitCode.no_message_mid_run("Codex CLI"));
    for (const k of ["redirect", "transcript", "inspect", "vajra_workflow"] as const) {
      expect(within(codex).getByRole("button", { name: C.capability[k] }).getAttribute("aria-disabled")).toBe("true");
    }
    // A capability that is off with no limits entry still says why, in this build's words.
    expect(within(codex).getAllByText(C.offFallback("Codex CLI")).length).toBe(2);
    // Claude has nothing off: no disabled control in its card.
    const claude = screen.getByRole("region", { name: "Claude Code" });
    expect(within(claude).queryAllByRole("button").filter((b) => b.getAttribute("aria-disabled") === "true")).toHaveLength(0);
  });

  it("words a known code in this app's words, and shows the core's own sentence for a code it does not know", async () => {
    const codex = LIST.providers[1];
    const odd = { ...codex, limits: [...codex.limits.filter((l: { control: string }) => l.control !== "message"), { control: "message", code: "no_message_new_kind", reason: "A reason from a newer core" }] };
    settings({ provider_list: { ...LIST, providers: [LIST.providers[0], odd] } });
    const card = await screen.findByRole("region", { name: "Codex CLI" });
    expect(within(card).getByText("A reason from a newer core")).toBeTruthy();
    expect(within(card).getByText(C.limitCode.no_redirect_mid_run("Codex CLI"))).toBeTruthy();
    expect(within(card).getByText(new RegExp(C.limitCode.stop_ends_process("Codex CLI").slice(0, 20)))).toBeTruthy();
  });

  it("chooses the agent per project through act_preferences_update; a not-installed one cannot be chosen", async () => {
    const f = settings({
      provider_list: codexMissing(),
      act_preferences_update: () => ({ receipt: { state: "done", action_id: "a" }, result: view(P, "claude", 1) }),
    });
    const select = (await screen.findByLabelText(C.agent)) as HTMLSelectElement;
    await waitFor(() => expect([...select.options].map((o) => o.textContent)).toEqual(["Claude Code", C.notInstalledOption("Codex CLI")]));
    expect([...select.options].find((o) => o.value === "codex")!.disabled).toBe(true);
    // The choice is scoped like every other preference: this project, or the global default.
    expect((screen.getByRole("combobox", { name: S.preferences.scope }) as HTMLSelectElement).value).toBe("project");
    expect(f.calls.filter((c) => c.cmd === "act_preferences_update")).toHaveLength(0);
  });

  it("saves codex for this project only, with the scope's revision", async () => {
    const f = settings({ act_preferences_update: () => ({ receipt: { state: "done", action_id: "a" }, result: view(P, "codex", 1) }) });
    const select = (await screen.findByLabelText(C.agent)) as HTMLSelectElement;
    await waitFor(() => expect([...select.options].map((o) => o.textContent)).toEqual(["Claude Code", C.untestedOption("Codex CLI")]));
    expect([...select.options].find((o) => o.value === "codex")!.disabled).toBe(false);
    fireEvent.change(select, { target: { value: "codex" } });
    fireEvent.click(screen.getByRole("button", { name: S.preferences.save }));
    await screen.findByText(S.preferences.saved);
    expect(f.calls.find((c) => c.cmd === "act_preferences_update")!.args).toMatchObject({ envelope: { project_id: P, expect_generation: 0 }, patch: { agent: "codex" }, reset: false });
  });

  it("keeps one provider choice: Models and capacity has no agent row", async () => {
    window.location.hash = "#/settings?section=models";
    setTransport(fakeIpc({ preferences_read: view() }).transport);
    render(<PreferencesProvider projectId={null}><Settings /></PreferencesProvider>);
    expect(await screen.findByLabelText(S.preferences.workers)).toBeTruthy();
    expect(screen.queryByLabelText(C.agent)).toBeNull();
  });

  it("says when the providers cannot be checked, offers a retry, and still lets the choice be made", async () => {
    let n = 0;
    const f = settings({ provider_list: () => (++n === 1 ? new Error("down") : LIST) });
    const problem = await screen.findByText(C.listFailed);
    expect(problem).toBeTruthy();
    const select = (await screen.findByLabelText(C.agent)) as HTMLSelectElement;
    expect([...select.options].map((o) => o.value)).toEqual(["claude", "codex"]);
    fireEvent.click(screen.getByRole("button", { name: S.states.retry }));
    await screen.findByRole("region", { name: "Codex CLI" });
    expect(f.calls.filter((c) => c.cmd === "provider_list")).toHaveLength(2);
  });

  it("reads another schema major as update needed and guesses nothing", async () => {
    settings({ provider_list: fx("providers.v2.json") });
    expect(await screen.findByText(C.updateNeeded)).toBeTruthy();
    expect(screen.queryByRole("region", { name: "Codex CLI" })).toBeNull();
  });

  it("drops an older provider answer that lands after Check again", async () => {
    let first!: (v: unknown) => void;
    let n = 0;
    settings({ provider_list: () => (++n === 1 ? new Promise((r) => { first = r; }) : codexMissing()) });
    await screen.findByLabelText(C.agent);
    fireEvent.click(screen.getByRole("button", { name: S.settings.recheck }));
    await screen.findByText(C.loginState.not_installed);
    await act(async () => first(LIST));
    expect(screen.getByText(C.loginState.not_installed)).toBeTruthy();
    expect(screen.queryByText(C.loginState.cannot_tell)).toBeNull();
  });
});

describe("Settings > Providers > Set up recheck (ADR-17 rule 7)", () => {
  const row = (installed: boolean) => ({
    id: "codex", name: "Codex CLI", state: installed ? "installed" : "manual", version: installed ? "0.167.1" : null,
    path: installed ? "/new/bin/codex" : null, reason_code: installed ? null : "source_not_verified",
    commands: [{ tool: "brew", command: "brew install --cask codex" }], docs_url: "https://github.com/openai/codex", can_sign_in: installed,
  });
  const setupList = (installed: boolean) => ({ ...SETUP_EMPTY, rows: [row(installed)] });

  it("Check again re-reads Set up, so a CLI installed with the vendor's steps gains Sign in without leaving the page", async () => {
    let n = 0;
    settings({ setup_list: () => setupList(++n > 1) });
    await screen.findByText(S.setup.state.manual);
    expect(screen.queryByRole("button", { name: S.setup.signIn("Codex CLI") })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: S.settings.recheck }));
    expect(await screen.findByRole("button", { name: S.setup.signIn("Codex CLI") })).toBeTruthy();
    expect(screen.getByText("/new/bin/codex")).toBeTruthy();
    expect(screen.queryByText(S.setup.state.manual)).toBeNull();
  });

  it("on entering Providers, waits for Set up (the PATH refresh) before the checks, so a CLI in a new bin folder is found by all of them", async () => {
    let refreshed = false;
    let answer!: (v: unknown) => void;
    const f = settings({
      setup_list: () => new Promise((r) => { answer = (v) => { refreshed = true; r(v); }; }),
      provider_list: () => (refreshed ? LIST : codexMissing()),
      preflight: () => (refreshed ? preflight : { rows: [{ name: "codex", ok: false, detail: "", fix: "" }], start_enabled: false }),
    });
    await waitFor(() => expect(f.calls.some((c) => c.cmd === "setup_list")).toBe(true));
    await new Promise((r) => setTimeout(r, 20));
    expect(f.calls.filter((c) => ["provider_list", "preflight", "provider_accounts"].includes(c.cmd))).toEqual([]);
    await act(async () => answer(setupList(true)));
    expect(await screen.findByText("0.9.0")).toBeTruthy(); // the agent card's Codex version: found on the refreshed PATH
    expect(screen.queryByText(C.loginState.not_installed)).toBeNull();
  });

  it("refreshes the PATH (setup_list) before the checks that depend on it", async () => {
    const f = settings({ setup_list: () => setupList(false) });
    await screen.findByText(S.setup.state.manual);
    const from = f.calls.length;
    fireEvent.click(screen.getByRole("button", { name: S.settings.recheck }));
    await waitFor(() => expect(f.calls.slice(from).map((c) => c.cmd)).toEqual(expect.arrayContaining(["setup_list", "preflight", "provider_list"])));
    const order = f.calls.slice(from).map((c) => c.cmd).filter((c) => ["setup_list", "preflight", "provider_list"].includes(c));
    expect(order[0]).toBe("setup_list");
  });
});

describe("Start and Resume use the chosen provider (FG-09)", () => {
  // Home starts through an action receipt (FG-11); the provider rides in the same input a direct start sends.
  const started = { receipt: { schema: "vajra.receipt/1", action_id: "act-x", project_id: "p_x", action: "start_run", target: "p_x", expect_generation: null, current_generation: null, state: "done", reason: null, detail: null, retry: "no", rev: 1, ts_ms: 1 }, result: { stream: "s", session_id: "u", run_id: "r1", campaign: "c1" } };
  const home = (agent: "claude" | "codex", replies: Record<string, unknown> = {}) => {
    const f = fakeIpc({ preflight, list_campaigns: { campaigns: [] }, resume_targets: { targets: {} }, provider_list: LIST, preferences_read: view(null, agent), act_start_run: started, ...replies });
    setTransport(f.transport);
    render(<PreferencesProvider projectId={null}><Home /></PreferencesProvider>);
    return f;
  };
  const go = async () => {
    await screen.findByText("2.1.0");
    fireEvent.change(screen.getByLabelText(S.home.repo), { target: { value: "/r" } });
    fireEvent.change(screen.getByLabelText(S.home.ask), { target: { value: "do it" } });
    fireEvent.click(screen.getByRole("button", { name: S.home.startButton }));
  };

  it("passes claude, and never runs the provider check for it (no idle subprocesses)", async () => {
    const f = home("claude");
    await go();
    await waitFor(() => expect(f.calls.some((c) => c.cmd === "act_start_run")).toBe(true));
    expect(f.calls.find((c) => c.cmd === "act_start_run")!.args).toMatchObject({ provider: "claude", cwd: "/r" });
    expect(f.calls.some((c) => c.cmd === "provider_list")).toBe(false);
  });

  it("passes codex, says so, and shows what is off with reasons; cannot_tell does not block", async () => {
    const f = home("codex");
    expect(await screen.findByText(C.runsOn("Codex CLI"))).toBeTruthy();
    const off = screen.getByRole("button", { name: C.capability.message });
    expect(off.getAttribute("aria-disabled")).toBe("true");
    await go();
    await waitFor(() => expect(f.calls.some((c) => c.cmd === "act_start_run")).toBe(true));
    expect(f.calls.find((c) => c.cmd === "act_start_run")!.args).toMatchObject({ provider: "codex" });
  });

  it("does not start a provider that is not installed, says why, and never swaps to another", async () => {
    const f = home("codex", { provider_list: codexMissing() });
    expect((await screen.findByText(C.startBlocked("Codex CLI"))).closest("[role=alert]")).toBeTruthy();
    await go();
    expect(screen.getByRole("button", { name: S.home.startButton }).getAttribute("aria-disabled")).toBe("true");
    expect(f.calls.some((c) => c.cmd === "act_start_run")).toBe(false);
  });

  it("turns Resume off with its reason when the run's provider is not installed", async () => {
    const f = fakeIpc({
      preflight, provider_list: codexMissing(), preferences_read: view(),
      list_campaigns: { campaigns: [{ slug: "c1", goal: "ship it", status: "active", passing: 1, total: 2 }] },
      resume_targets: { targets: { c1: { session_id: "s", run_id: "r", cwd: "/r", provider: "codex" } } },
    });
    setTransport(f.transport);
    render(<PreferencesProvider projectId={null}><Home /></PreferencesProvider>);
    const resume = await screen.findByRole("button", { name: S.home.resume });
    await waitFor(() => expect(resume.getAttribute("aria-disabled")).toBe("true"));
    expect(document.getElementById(resume.getAttribute("aria-describedby") ?? "")?.textContent).toBe(C.resumeOff("Codex CLI"));
    fireEvent.click(resume);
    expect(f.calls.some((c) => c.cmd === "resume_run")).toBe(false);
  });
});

describe("The run's provider in Run and Crew (FG-09 G6)", () => {
  it("Run names a codex run's provider and shows its off controls with reasons", async () => {
    window.location.hash = "#/run?slug=c1";
    setTransport(fakeIpc({
      evidence_list: { slug: "c1", stale: false, rows: [] }, run_overview: new Error("none"), provider_list: LIST,
      resume_targets: { targets: { c1: { session_id: "s", run_id: "r", cwd: "/r", provider: "codex" } } },
    }).transport);
    render(<Run />);
    const strip = await screen.findByRole("region", { name: C.runsOn("Codex CLI") });
    expect(within(strip).getByRole("button", { name: C.capability.redirect }).getAttribute("aria-disabled")).toBe("true");
    expect(within(strip).getByText(C.limitCode.no_redirect_mid_run("Codex CLI"))).toBeTruthy();
  });

  it("Run says nothing extra for a claude run and does not run the provider check", async () => {
    window.location.hash = "#/run?slug=c1";
    const f = fakeIpc({ evidence_list: { slug: "c1", stale: false, rows: [] }, run_overview: new Error("none"), provider_list: LIST, resume_targets: { targets: { c1: { session_id: "s", run_id: "r", cwd: "/r" } } } });
    setTransport(f.transport);
    render(<Run />);
    await waitFor(() => expect(f.calls.some((c) => c.cmd === "resume_targets")).toBe(true));
    await screen.findByText(S.run.none);
    expect(screen.queryByRole("region", { name: /Runs on/ })).toBeNull();
    expect(f.calls.some((c) => c.cmd === "provider_list")).toBe(false);
  });

  it("Crew names a codex run's provider with its off controls, even when the crew cannot be read", async () => {
    window.location.hash = "#/crew?slug=c1&id=r1";
    setTransport(fakeIpc({
      active_runs: { count: 0, runs: [] }, crew: new Error("no crew"), provider_list: LIST,
      resume_targets: { targets: { c1: { session_id: "s", run_id: "r1", cwd: "/r", provider: "codex" } } },
    }).transport);
    render(<Crew />);
    const strip = await screen.findByRole("region", { name: C.runsOn("Codex CLI") });
    expect(within(strip).getByRole("button", { name: C.capability.vajra_workflow }).getAttribute("aria-disabled")).toBe("true");
    expect(within(strip).getByText(C.limitCode.no_vajra_workflow("Codex CLI"))).toBeTruthy();
  });

  it("Crew drops a late provider answer for the run it has moved away from", async () => {
    window.location.hash = "#/crew?slug=c1&id=r1";
    // One resolver per request, so the answer goes to c1's own request, not whichever asked last.
    const asked: ((v: unknown) => void)[] = [];
    setTransport(fakeIpc({
      active_runs: { count: 0, runs: [] }, crew: new Error("no crew"), provider_list: LIST,
      resume_targets: () => new Promise((r) => { asked.push(r); }),
    }).transport);
    render(<Crew />);
    await waitFor(() => expect(asked).toHaveLength(1));
    // jsdom delivers `hashchange` as a later task: wait until Crew has really moved to r2 (and asked for it) before
    // answering, so the answer is late by construction rather than by timing.
    act(() => { window.location.hash = "#/crew?slug=c2&id=r2"; });
    await screen.findByText("r2");
    await waitFor(() => expect(asked).toHaveLength(2));
    await act(async () => asked[0]({ targets: { c1: { session_id: "s", run_id: "r1", cwd: "/r", provider: "codex" } } }));
    expect(screen.queryByRole("region", { name: C.runsOn("Codex CLI") })).toBeNull();
  });
});
