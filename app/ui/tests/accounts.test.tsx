// Multi-provider accounts in Settings > Providers and the project's agents, against fixtures (the core commands land later).
// No real key is used: FAKE_KEY is a made-up string, and the only transport is a recording fake.
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { ACCOUNTS as FIXTURE_ACCOUNTS, AGENTS, USAGE } from "../src/accountsFixture";
import { BASE_URL_SAVE_WIRED, allowedChoices, toggleAllowed, baseUrlProblem, isNewModel, modelLine, parseAccounts, parseAgents, parseUsage, warnWindow } from "../src/accountsContract";
import type { Account } from "../src/accountsContract";
import { setTransport } from "../src/ipc";
import { PreferencesProvider } from "../src/preferences";
import { DEFAULT_PREFERENCES } from "../src/preferenceContract";
import { CurrentProject } from "../src/projects/current";
import { ProviderIcon } from "../src/screens/ProviderIcon";
import { BaseUrl } from "../src/screens/Accounts";
import { Settings } from "../src/screens/settings/Settings";
import { S } from "../strings";
import { fakeIpc } from "./fakeIpc";

const C = S.accounts;
const A = S.agents;
const FAKE_KEY = "fake-test-key-0000-not-a-secret";
const P = "p_01JC5X8Z3A9K2M4N6P7Q8R9S0T";
const project = { schema: "vajra.project/1", id: P, name: "harbor-api", root: "/src/api", created: null, archived: null, rev: 0 };
const clone = <T,>(v: T): T => JSON.parse(JSON.stringify(v));
const view = () => ({
  schema: "vajra.preferences/1", rev: 0, project_id: null, caps: { active_runs: 1, workers_per_run: 16, editable: false },
  global: { ...DEFAULT_PREFERENCES }, overrides: {}, effective: { ...DEFAULT_PREFERENCES },
  sources: Object.fromEntries(Object.keys(DEFAULT_PREFERENCES).map((k) => [k, "global"])),
});
const providerList = { schema: "vajra.providers/1", providers: [] };

beforeAll(() => {
  // jsdom has no modal dialog: model showModal/close the way browsers do (as base.test.tsx does).
  HTMLDialogElement.prototype.showModal = function (this: HTMLDialogElement) { this.setAttribute("open", ""); };
  HTMLDialogElement.prototype.close = function (this: HTMLDialogElement) { this.removeAttribute("open"); this.dispatchEvent(new Event("close")); };
});
afterEach(() => { cleanup(); setTransport(); window.location.hash = ""; });

function settings(replies: Record<string, unknown> = {}, inProject = true) {
  window.location.hash = "#/settings";
  let accounts = clone(FIXTURE_ACCOUNTS);
  const f = fakeIpc({
    preflight: { rows: [], start_enabled: true }, provider_list: providerList, preferences_read: view,
    provider_accounts: () => accounts, provider_usage: USAGE, provider_agents: clone(AGENTS),
    provider_key_save: () => { accounts = { ...accounts, accounts: accounts.accounts.map((a) => a.id === "openrouter" ? { ...a, state: "key_saved" } : a) }; return null; },
    provider_key_replace: () => null, provider_key_remove: () => null, provider_base_url_save: () => null,
    provider_agents_save: (a?: Record<string, unknown>) => ({ schema: AGENTS.schema, ...(a?.agents as object) }),
    ...replies,
  });
  setTransport(f.transport);
  render(
    <CurrentProject.Provider value={{ project: inProject ? project : null, campaigns: [], focus: null, runId: null }}>
      <PreferencesProvider projectId={inProject ? P : null}><Settings /></PreferencesProvider>
    </CurrentProject.Provider>,
  );
  return f;
}
const account = (id: string) => screen.findByRole("listitem", { name: id });

describe("accounts contract", () => {
  it("reads the fixture, and keeps only the fields it names (a stray secret in an answer never reaches a component)", () => {
    const withSecret = clone(FIXTURE_ACCOUNTS) as any;
    withSecret.accounts[2].credential.key = FAKE_KEY;
    withSecret.accounts[2].api_key = FAKE_KEY;
    const r = parseAccounts(withSecret);
    expect(r.ok).toBe(true);
    expect(JSON.stringify(r)).not.toContain(FAKE_KEY);
  });
  it("an unknown state reads as cannot_tell; another major reads as update needed; a bad account fails the whole answer", () => {
    const odd = clone(FIXTURE_ACCOUNTS) as any;
    odd.accounts[0].state = "wibble";
    const r = parseAccounts(odd);
    expect(r.ok && r.value.accounts[0].state).toBe("cannot_tell");
    expect(parseAccounts({ ...FIXTURE_ACCOUNTS, schema: "vajra.provider_accounts/2" })).toMatchObject({ ok: false, reason: "unsupported_version" });
    const bad = clone(FIXTURE_ACCOUNTS) as any;
    bad.accounts[1].label = "";
    expect(parseAccounts(bad).ok).toBe(false);
    expect(parseUsage({ ...USAGE, schema: "vajra.provider_usage/9" })).toMatchObject({ ok: false, reason: "unsupported_version" });
    expect(parseAgents({ ...AGENTS, roles: { janitor: AGENTS.default } }).ok).toBe(false);
  });
  it("the 80% line: at 80 it warns, below it does not, and a window that reports nothing never warns", () => {
    const u = (used_pct: number | null) => ({ account: "x", read_at: null, windows: [{ kind: "weekly" as const, used_pct, resets_at: null, used_usd: null, limit_usd: null }] });
    expect(warnWindow(u(80))).not.toBeNull();
    expect(warnWindow(u(79))).toBeNull();
    expect(warnWindow(u(null))).toBeNull();
  });
  it("a gateway base URL is a web address with no user:password in it", () => {
    expect(baseUrlProblem("https://openrouter.ai/api/v1")).toBeNull();
    expect(baseUrlProblem("http://localhost:11434")).toBeNull();
    expect(baseUrlProblem("")).toBe("empty");
    expect(baseUrlProblem("http://127.0.0.1:8080")).toBeNull();
    expect(baseUrlProblem("http://[::1]:8080/v1")).toBeNull();
    // The gateway key is sent to this address: plain http to any other host would send it in the clear.
    expect(baseUrlProblem("http://openrouter.ai/api/v1")).toBe("insecure");
    expect(baseUrlProblem("http://10.0.0.5")).toBe("insecure");
    expect(baseUrlProblem("http://localhost.evil.example")).toBe("insecure");
    expect(baseUrlProblem("ftp://x")).toBe("scheme");
    expect(baseUrlProblem("not a url")).toBe("scheme");
    expect(baseUrlProblem("https://user:hunter2@host/")).toBe("credentials");
  });
});

describe("Settings > Providers > Accounts", () => {
  it("lists every account with its state, billing label and where its workers run", async () => {
    settings();
    const claude = await account("Claude Code");
    expect(within(claude).getByText(C.state.signed_in)).toBeTruthy();
    expect(within(claude).getByText(C.billing.subscription("Max plan, s…@gmail.com"))).toBeTruthy();
    expect(within(claude).getByText(C.guarded)).toBeTruthy();
    const codex = await account("Codex CLI");
    expect(within(codex).getByText(C.billing.unknown)).toBeTruthy();
    expect(within(codex).getByText(C.outsideGuard)).toBeTruthy();
    const or = await account("OpenRouter");
    expect(within(or).getByText(C.state.key_saved)).toBeTruthy();
    expect(within(or).getByText(C.billing.gateway("OpenRouter key"))).toBeTruthy();
    expect(within(or).getByText(C.runsOn("Codex CLI"))).toBeTruthy();
    expect(screen.getByText(C.count(3))).toBeTruthy();
  });

  it("the override warning says the key bills instead of the plan", async () => {
    settings();
    const codex = await account("Codex CLI");
    expect(within(codex).getByText(C.overrideWarning({ name: "OPENAI_API_KEY", found_in: "cli_config" }))).toBeTruthy();
    expect(within(await account("Claude Code")).queryByText(/bills instead/)).toBeNull();
  });

  it("usage: the 80% chip shows on the account past the line only; an account that reports nothing says so; credit shows its limit", async () => {
    settings();
    const claude = await account("Claude Code");
    const weekly = within(claude).getByRole("meter", { name: "Claude Code Weekly" });
    expect(weekly.getAttribute("aria-valuenow")).toBe("84");
    expect(within(claude).getAllByText(C.used(84))).toHaveLength(2); // the figure and the warning chip
    expect(within(claude).getAllByText(C.used(62))).toHaveLength(1); // the 5-hour window: figure only, no chip
    expect(within(claude).getByText(C.resets("Thu 09:00"))).toBeTruthy();
    expect(within(await account("Codex CLI")).getByText(C.notReported)).toBeTruthy();
    const or = await account("OpenRouter");
    expect(within(or).getByText(C.credit(3.1, 10))).toBeTruthy();
    expect(within(or).getByText(C.creditLimitAt)).toBeTruthy();
    expect(within(or).queryByText(C.used(31))).toBeNull(); // 31% is below the line: no chip
  });

  it("a usage answer that fails leaves the accounts up, with the meters saying not reported", async () => {
    settings({ provider_usage: new Error("no") });
    expect(within(await account("Claude Code")).getByText(C.notReported)).toBeTruthy();
  });

  it("every model row names its provider; a bare model id is a bug", async () => {
    settings();
    await account("OpenRouter");
    const lines = [...document.querySelectorAll("[data-model-line]")].map((n) => n.textContent ?? "");
    expect(lines.length).toBeGreaterThanOrEqual(2);
    const ids = FIXTURE_ACCOUNTS.accounts.flatMap((a) => a.models.map((m) => m.id));
    for (const text of lines) {
      expect(text).toMatch(/^.+ · .+$/);
      expect(ids).not.toContain(text);
      expect(ids.some((id) => text.startsWith(id))).toBe(false);
    }
    expect(lines).toContain("DeepSeek V3.2 · OpenRouter");
    // The helper itself: a model the core gave no name for still reads "id · provider", never the id alone.
    const or = parseAccounts(FIXTURE_ACCOUNTS);
    const acc = (or.ok ? or.value.accounts[2] : null) as Account;
    expect(modelLine(acc, "unlisted/model-1", C.defaultModel)).toBe("unlisted/model-1 · OpenRouter");
    expect(modelLine(acc, null, C.defaultModel)).toBe(`${C.defaultModel} · OpenRouter`);
  });

  it("OpenRouter models are labelled New and carry their tool-use check; other accounts' rows are not", async () => {
    settings();
    const or = await account("OpenRouter");
    expect(within(or).getAllByText(C.newBadge)).toHaveLength(2);
    expect(within(or).getByText(C.toolCheck.passed)).toBeTruthy();
    expect(within(or).getByText(C.toolCheck.unchecked)).toBeTruthy();
    const accs = parseAccounts(FIXTURE_ACCOUNTS);
    expect(accs.ok && accs.value.accounts.map(isNewModel)).toEqual([false, false, true]);
  });

  it("a failed answer shows a problem, not a blank panel; another major says to update", async () => {
    settings({ provider_accounts: { ...FIXTURE_ACCOUNTS, schema: "vajra.provider_accounts/2" } });
    expect((await screen.findAllByText(C.updateNeeded)).length).toBeGreaterThan(0);
  });
});

describe("the OpenRouter key", () => {
  const open = async (replies: Record<string, unknown> = {}) => {
    const f = settings(replies);
    const or = await account("OpenRouter");
    fireEvent.click(within(or).getByRole("button", { name: `${C.replace}: OpenRouter` }));
    return f;
  };

  it("the sheet says the key goes to the Keychain; it is a password field with no stored value", async () => {
    await open();
    const field = screen.getByLabelText(C.keyLabel) as HTMLInputElement;
    expect(field.type).toBe("password");
    expect(field.value).toBe("");
    expect(screen.getByText(C.keyHint)).toBeTruthy();
    expect(screen.getByText(C.keyLimit)).toBeTruthy();
    expect(screen.getByText(C.keyHarness(true))).toBeTruthy();
  });

  it("Replace on a saved account sends the key to provider_key_replace (never Save, which refuses a second key), then the field and the page hold no trace of it", async () => {
    const f = await open();
    const field = screen.getByLabelText(C.keyLabel) as HTMLInputElement;
    fireEvent.change(field, { target: { value: FAKE_KEY } });
    fireEvent.click(screen.getByRole("button", { name: C.save }));
    await waitFor(() => expect(f.calls.filter((c) => c.cmd === "provider_key_replace")).toHaveLength(1));
    expect(f.calls.find((c) => c.cmd === "provider_key_replace")?.args).toEqual({ accountId: "openrouter", key: FAKE_KEY });
    expect(f.calls.some((c) => c.cmd === "provider_key_save")).toBe(false);
    await waitFor(() => expect(f.calls.filter((c) => c.cmd === "provider_accounts").length).toBeGreaterThan(1));
    await waitFor(() => expect((screen.getByLabelText(C.keyLabel) as HTMLInputElement).value).toBe(""));
    expect(document.body.innerHTML).not.toContain(FAKE_KEY);
    expect(JSON.stringify(f.calls.filter((c) => c.cmd !== "provider_key_replace"))).not.toContain(FAKE_KEY);
    const or = await account("OpenRouter");
    fireEvent.click(within(or).getByRole("button", { name: `${C.replace}: OpenRouter` }));
    expect((screen.getByLabelText(C.keyLabel) as HTMLInputElement).value).toBe("");
  });

  it("Add on an account with no key sends the key to provider_key_save once, and nothing else carries it", async () => {
    const none = clone(FIXTURE_ACCOUNTS) as any;
    none.accounts = none.accounts.map((a: any) => (a.id === "openrouter" ? { ...a, state: "key_missing", credential: { source: "api_key", saved: false } } : a));
    const f = settings({ provider_accounts: none });
    const or = await account("OpenRouter");
    // No key yet: Add, and no Remove.
    expect(within(or).queryByRole("button", { name: `${C.remove}: OpenRouter` })).toBeNull();
    fireEvent.click(within(or).getByRole("button", { name: `${C.add}: OpenRouter` }));
    fireEvent.change(screen.getByLabelText(C.keyLabel), { target: { value: FAKE_KEY } });
    fireEvent.click(screen.getByRole("button", { name: C.save }));
    await waitFor(() => expect(f.calls.filter((c) => c.cmd === "provider_key_save")).toHaveLength(1));
    expect(f.calls.find((c) => c.cmd === "provider_key_save")?.args).toEqual({ accountId: "openrouter", key: FAKE_KEY });
    expect(f.calls.some((c) => c.cmd === "provider_key_replace")).toBe(false);
    expect(JSON.stringify(f.calls.filter((c) => c.cmd !== "provider_key_save"))).not.toContain(FAKE_KEY);
  });

  it("Cancel drops what was typed; Save is off while the field is empty", async () => {
    const f = await open();
    expect(screen.getByRole("button", { name: C.save }).getAttribute("aria-disabled")).toBe("true");
    fireEvent.change(screen.getByLabelText(C.keyLabel), { target: { value: FAKE_KEY } });
    fireEvent.click(screen.getByRole("button", { name: C.cancel }));
    expect((screen.getByLabelText(C.keyLabel) as HTMLInputElement).value).toBe("");
    expect(f.calls.some((c) => c.cmd === "provider_key_save" || c.cmd === "provider_key_replace")).toBe(false);
  });

  it("a failed replace says so and still never shows the key", async () => {
    await open({ provider_key_replace: new Error("x") });
    fireEvent.change(screen.getByLabelText(C.keyLabel), { target: { value: FAKE_KEY } });
    fireEvent.click(screen.getByRole("button", { name: C.save }));
    const sheet = screen.getByLabelText(C.keyLabel).closest("dialog") as HTMLElement;
    expect((await within(sheet).findByRole("alert")).textContent).toContain(C.keyFailed);
    expect(document.body.textContent).not.toContain(FAKE_KEY);
  });

  it("Remove asks first, and only the confirm removes the key", async () => {
    const f = settings();
    const or = await account("OpenRouter");
    fireEvent.click(within(or).getByRole("button", { name: `${C.remove}: OpenRouter` }));
    expect(f.calls.some((c) => c.cmd === "provider_key_remove")).toBe(false);
    expect(screen.getByText(C.removeBody)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: C.removeConfirm }));
    await waitFor(() => expect(f.calls.find((c) => c.cmd === "provider_key_remove")?.args).toEqual({ accountId: "openrouter" }));
  });

  it("with no gateway account there is no Add button, so a key is never sent to an account id the core may not have", async () => {
    const none = clone(FIXTURE_ACCOUNTS) as any;
    none.accounts = none.accounts.slice(0, 2);
    const f = settings({ provider_accounts: none });
    await account("Claude Code");
    expect(screen.queryByRole("button", { name: C.add })).toBeNull();
    expect(f.calls.some((c) => c.cmd === "provider_key_save")).toBe(false);
  });

  it("with no key saved the account offers Add, not Replace or Remove", async () => {
    const noKey = clone(FIXTURE_ACCOUNTS) as any;
    noKey.accounts[2] = { ...noKey.accounts[2], state: "key_missing", credential: { source: "api_key", saved: false } };
    settings({ provider_accounts: noKey });
    const or = await account("OpenRouter");
    expect(within(or).getByRole("button", { name: `${C.add}: OpenRouter` })).toBeTruthy();
    expect(within(or).queryByRole("button", { name: /Remove/ })).toBeNull();
  });
});

describe("advanced: base URL", () => {
  it("is not offered on any row while the core has no command to save it (no control that always errors)", async () => {
    const f = settings();
    for (const name of ["Claude Code", "Codex CLI", "OpenRouter"]) {
      const row = await account(name);
      expect(within(row).queryByText(C.advanced)).toBeNull();
      expect(within(row).queryByLabelText(C.baseUrlLabel)).toBeNull();
    }
    expect(BASE_URL_SAVE_WIRED).toBe(false);
    expect(f.calls.some((c) => c.cmd === "provider_base_url_save")).toBe(false);
  });

  // The box itself stays tested, so turning BASE_URL_SAVE_WIRED on later is a one-line change.
  it("the box refuses a URL with a user name or password in it, and saves a plain one", async () => {
    const f = fakeIpc({ provider_base_url_save: null });
    setTransport(f.transport);
    const or = clone(FIXTURE_ACCOUNTS).accounts.find((a) => a.id === "openrouter") as Account;
    render(<ul><li aria-label="box"><BaseUrl account={or} onSaved={() => {}} /></li></ul>);
    const box = screen.getByRole("listitem", { name: "box" });
    const field = within(box).getByLabelText(C.baseUrlLabel);
    fireEvent.change(field, { target: { value: "https://user:hunter2@host/v1" } });
    fireEvent.click(within(box).getByRole("button", { name: C.baseUrlSave }));
    expect((await within(box).findByRole("alert")).textContent).toBe(C.baseUrlProblem.credentials);
    expect(f.calls.some((c) => c.cmd === "provider_base_url_save")).toBe(false);
    fireEvent.change(field, { target: { value: "https://openrouter.ai/api/v1" } });
    fireEvent.click(within(box).getByRole("button", { name: C.baseUrlSave }));
    await waitFor(() => expect(f.calls.find((c) => c.cmd === "provider_base_url_save")?.args).toEqual({ accountId: "openrouter", baseUrl: "https://openrouter.ai/api/v1" }));
  });
});

describe("toggleAllowed: row kind x covered/uncovered x checked/unchecked", () => {
  const M = ["m1", "m2"];
  const e = (model: string | null) => ({ account: "a", model });
  const other = { account: "z", model: null };
  const out = (allowed: Array<{ account: string; model: string | null }>, model: string | null) => toggleAllowed(allowed, e(model), M);
  it("covered: the default row (null) off keeps every model, as explicit entries", () => {
    expect(out([e(null), other], null)).toEqual([other, e("m1"), e("m2")]);
  });
  it("covered: a model row off keeps the other models and drops the account-wide entry", () => {
    expect(out([e(null), other], "m1")).toEqual([other, e("m2")]);
  });
  it("covered with some explicit entries already: no duplicates", () => {
    expect(out([e(null), e("m2")], "m1")).toEqual([e("m2")]);
  });
  it("uncovered, checked: only that entry goes", () => {
    expect(out([e("m1"), e("m2"), other], "m1")).toEqual([e("m2"), other]);
  });
  it("uncovered, unchecked model row: only that entry is added", () => {
    expect(out([e("m1")], "m2")).toEqual([e("m1"), e("m2")]);
  });
  it("uncovered, unchecked default row: adds the account-wide entry and drops the redundant model entries", () => {
    expect(out([e("m1"), other], null)).toEqual([other, e(null)]);
  });
  it("never touches another account", () => {
    for (const m of [null, "m1"]) expect(out([e(null), other], m)).toContainEqual(other);
  });
});

describe("Project scope: the default, role defaults and the allowed set", () => {
  it("an account allowed at null offers every model on it; a concrete allowance offers only that model", () => {
    const r = parseAccounts(FIXTURE_ACCOUNTS);
    if (!r.ok) throw new Error("fixture accounts");
    const list = r.value.accounts;
    const or = list.find((a) => a.id === "openrouter")!;
    expect(or.models.length).toBeGreaterThan(1);
    const all = allowedChoices(list, [{ account: "openrouter", model: null }]).map((c) => c.model);
    expect(all).toEqual(or.models.map((m) => m.id)); // no bogus "default" entry for a gateway
    const one = allowedChoices(list, [{ account: "openrouter", model: or.models[0].id }]).map((c) => c.model);
    expect(one).toEqual([or.models[0].id]);
    const cli = allowedChoices(list, [{ account: "claude", model: null }]);
    expect(cli[0].model).toBeNull();
    expect(cli.length).toBe(1 + list.find((a) => a.id === "claude")!.models.length);
  });

  const section = async () => screen.findByRole("region", { name: "harbor-api" });
  const options = (select: HTMLElement) => [...select.querySelectorAll("option")].map((o) => o.textContent);

  it("a project allowing the gateway account at null lists its models, and a saved concrete model is not stale", async () => {
    const allowed = [...AGENTS.allowed.filter((a) => a.account !== "openrouter"), { account: "openrouter", model: null }];
    settings({ provider_agents: { ...clone(AGENTS), allowed } });
    const s = await section();
    const docs = within(s).getByLabelText(A.pickLabel(A.role.docs)) as HTMLSelectElement;
    expect(docs.value).toBe("openrouter|deepseek/deepseek-v3.2");
    expect(docs.querySelector("option[disabled]")).toBeNull(); // not shown as stale
    expect(options(docs)).toContain("DeepSeek V3.2 · OpenRouter");
    expect(options(docs)).toContain("Grok 4 · OpenRouter"); // every model on the account is in the ceiling
  });

  it("reads the project's agents; every option reads Model · provider; only allowed choices are offered", async () => {
    settings();
    const s = await section();
    const def = within(s).getByLabelText(A.pickLabel(A.defaultAgent)) as HTMLSelectElement;
    expect(def.value).toBe("claude|");
    expect(options(def)).toEqual([`${C.defaultModel} · Claude Code`, `${C.defaultModel} · Codex CLI`, "DeepSeek V3.2 · OpenRouter"]);
    for (const sel of within(s).getAllByRole("combobox")) {
      for (const o of options(sel)) if (o !== A.useDefault && o !== A.effortDefault && !["low", "medium", "high"].includes(o ?? "")) expect(o).toMatch(/^.+ · .+$/);
    }
    // Grok 4 is not allowed: no picker offers it, and its checkbox is off.
    expect(within(s).queryByRole("option", { name: "Grok 4 · OpenRouter" })).toBeNull();
    expect((within(s).getByRole("checkbox", { name: /Grok 4 · OpenRouter/ }) as HTMLInputElement).checked).toBe(false);
    const docs = within(s).getByLabelText(A.pickLabel(A.role.docs)) as HTMLSelectElement;
    expect(docs.value).toBe("openrouter|deepseek/deepseek-v3.2");
    expect((within(s).getByLabelText(A.pickLabel(A.role.backend)) as HTMLSelectElement).value).toBe("");
  });

  it("allowing a model offers it in the pickers; one a default still uses cannot be turned off", async () => {
    settings();
    const s = await section();
    const claude = within(s).getByRole("checkbox", { name: new RegExp(`${C.defaultModel} · Claude Code`) }) as HTMLInputElement;
    expect(claude.disabled).toBe(true); // the project default
    expect(claude.getAttribute("aria-describedby")).toBeTruthy();
    expect(document.getElementById(claude.getAttribute("aria-describedby") ?? "")?.textContent).toBe(A.inUse);
    fireEvent.click(within(s).getByRole("checkbox", { name: /Grok 4 · OpenRouter/ }));
    expect(options(within(s).getByLabelText(A.pickLabel(A.role.backend)))).toContain("Grok 4 · OpenRouter");
  });

  it("a saved pick whose account is gone stays visible as a disabled option, so what is shown is what Save stores", async () => {
    const gone = clone(AGENTS);
    gone.roles.docs = { account: "retired", model: "old/model-1", effort: null };
    settings({ provider_agents: gone });
    const s = await section();
    const docs = within(s).getByLabelText(A.pickLabel(A.role.docs)) as HTMLSelectElement;
    expect(docs.value).toBe("retired|old/model-1");
    const opt = [...docs.options].find((o) => o.value === "retired|old/model-1");
    expect(opt?.disabled).toBe(true);
    expect(opt?.textContent).toBe("old/model-1 · retired");
  });

  it("effort is offered only where the account has an effort setting", async () => {
    const noEffort = clone(FIXTURE_ACCOUNTS) as any;
    noEffort.accounts[0].efforts = [];
    settings({ provider_accounts: noEffort });
    const s = await section();
    expect(within(s).queryByLabelText(A.effortLabel(A.defaultAgent))).toBeNull();
    expect(within(s).getAllByText(A.noEffort).length).toBeGreaterThan(0);
    expect(within(s).getByLabelText(A.effortLabel(A.role.tests))).toBeTruthy();
  });

  it("unchecking one model under an account-wide allowance narrows it to the other models; a model a default uses stays locked", async () => {
    const allowed = [...AGENTS.allowed.filter((a) => a.account !== "openrouter"), { account: "openrouter", model: null }];
    const f = settings({ provider_agents: { ...clone(AGENTS), allowed } });
    const s = await section();
    const box = (name: string) => within(s).getByText(name, { selector: "[data-model-line]" }).closest("label")!.querySelector("input") as HTMLInputElement;
    expect(box("DeepSeek V3.2 · OpenRouter").disabled).toBe(true); // the Docs role uses it
    expect(box("Grok 4 · OpenRouter").checked).toBe(true);
    expect(box("Grok 4 · OpenRouter").disabled).toBe(false);
    fireEvent.click(box("Grok 4 · OpenRouter"));
    expect(box("Grok 4 · OpenRouter").checked).toBe(false);
    fireEvent.click(within(s).getByRole("button", { name: A.save }));
    await waitFor(() => expect(f.calls.some((c) => c.cmd === "provider_agents_save")).toBe(true));
    const sent = f.calls.find((c) => c.cmd === "provider_agents_save")?.args as { agents: { allowed: Array<{ account: string; model: string | null }> } };
    expect(sent.agents.allowed.filter((a) => a.account === "openrouter")).toEqual([{ account: "openrouter", model: "deepseek/deepseek-v3.2" }]);
    expect(sent.agents.allowed.filter((a) => a.account !== "openrouter")).toEqual(AGENTS.allowed.filter((a) => a.account !== "openrouter"));
  });

  it("unchecking a CLI login's default under an account-wide allowance keeps its models allowed", async () => {
    const accounts = clone(FIXTURE_ACCOUNTS) as any;
    const codex = accounts.accounts.find((a: any) => a.id === "codex");
    codex.models = [{ id: "gpt-x", family: "gpt", name: "GPT X", tool_check: "passed" }, { id: "gpt-y", family: "gpt", name: "GPT Y", tool_check: "passed" }];
    const agents = clone(AGENTS);
    agents.roles = {}; agents.default = { account: "claude", model: null, effort: null }; // nothing uses Codex
    const f = settings({ provider_accounts: accounts, provider_agents: agents });
    const s = await section();
    const box = (name: string) => within(s).getByText(name, { selector: "[data-model-line]" }).closest("label")!.querySelector("input") as HTMLInputElement;
    expect(box("GPT X · Codex CLI").checked).toBe(true);
    expect(box("GPT X · Codex CLI").disabled).toBe(true); // under the default row: turn the default off first
    fireEvent.click(box(`${C.defaultModel} · Codex CLI`));
    expect(box("GPT X · Codex CLI").disabled).toBe(false);
    expect(box(`${C.defaultModel} · Codex CLI`).checked).toBe(false);
    expect(box("GPT X · Codex CLI").checked).toBe(true);
    expect(box("GPT Y · Codex CLI").checked).toBe(true);
    fireEvent.click(within(s).getByRole("button", { name: A.save }));
    await waitFor(() => expect(f.calls.some((c) => c.cmd === "provider_agents_save")).toBe(true));
    const sent = f.calls.find((c) => c.cmd === "provider_agents_save")?.args as { agents: { allowed: Array<{ account: string; model: string | null }> } };
    expect(sent.agents.allowed.filter((a) => a.account === "codex")).toEqual([{ account: "codex", model: "gpt-x" }, { account: "codex", model: "gpt-y" }]);
  });

  it("Save sends the edited defaults and the allowed set; nothing is sent before", async () => {
    const f = settings();
    const s = await section();
    expect(within(s).getByRole("button", { name: A.save }).getAttribute("aria-disabled")).toBe("true");
    fireEvent.change(within(s).getByLabelText(A.pickLabel(A.role.backend)), { target: { value: "codex|" } });
    fireEvent.change(within(s).getByLabelText(A.effortLabel(A.role.backend)), { target: { value: "high" } });
    expect(f.calls.some((c) => c.cmd === "provider_agents_save")).toBe(false);
    fireEvent.click(within(s).getByRole("button", { name: A.save }));
    await waitFor(() => expect(f.calls.find((c) => c.cmd === "provider_agents_save")).toBeTruthy());
    const sent = f.calls.find((c) => c.cmd === "provider_agents_save")?.args as { projectId: string; agents: { roles: Record<string, unknown>; allowed: unknown[] } };
    expect(sent.projectId).toBe(P);
    expect(sent.agents.roles.backend).toEqual({ account: "codex", model: null, effort: "high" });
    expect(sent.agents.allowed).toEqual(AGENTS.allowed);
    expect((await within(s).findByRole("status")).textContent).toBe(A.saved);
  });

  it("changing the account clears an effort the new account does not take; Discard restores the saved agents", async () => {
    settings();
    const s = await section();
    const tests = within(s).getByLabelText(A.pickLabel(A.role.tests)) as HTMLSelectElement;
    fireEvent.change(tests, { target: { value: "openrouter|deepseek/deepseek-v3.2" } });
    expect((within(s).getByLabelText(A.effortLabel(A.role.tests)) as HTMLSelectElement).value).toBe("medium"); // OpenRouter takes medium too
    fireEvent.click(within(s).getByRole("button", { name: A.discard }));
    expect((within(s).getByLabelText(A.pickLabel(A.role.tests)) as HTMLSelectElement).value).toBe("codex|");
  });

  it("a failed save says so and keeps the draft", async () => {
    settings({ provider_agents_save: new Error("x") });
    const s = await section();
    fireEvent.change(within(s).getByLabelText(A.pickLabel(A.role.backend)), { target: { value: "codex|" } });
    fireEvent.click(within(s).getByRole("button", { name: A.save }));
    expect((await within(s).findByRole("alert")).textContent).toContain(A.saveFailed);
    expect((within(s).getByLabelText(A.pickLabel(A.role.backend)) as HTMLSelectElement).value).toBe("codex|");
  });

  it("with no project there is no project section", async () => {
    settings({}, false);
    await account("Claude Code");
    expect(screen.queryByRole("region", { name: "harbor-api" })).toBeNull();
  });
});

describe("provider icons", () => {
  it("a known provider shows its monogram, a gateway model its own tile with the gateway badge, and an unknown family the fallback icon", () => {
    const { container, rerender } = render(<ProviderIcon family="claude" />);
    expect(container.textContent).toBe("CC");
    rerender(<ProviderIcon family="deepseek" via="openrouter" />);
    expect(container.textContent).toBe("DSOR");
    rerender(<ProviderIcon family="some-new-family" />);
    expect(container.textContent?.trim()).toBe("");
    expect(container.querySelector("svg")).toBeTruthy();
    expect(container.querySelector("[data-family]")?.getAttribute("data-family")).toBe("unknown");
  });
  it("is decorative: hidden from assistive tech (the name is always written beside it)", () => {
    const { container } = render(<ProviderIcon family="grok" />);
    expect(container.firstElementChild?.getAttribute("aria-hidden")).toBe("true");
  });
});
