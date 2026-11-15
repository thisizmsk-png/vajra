// The TypeScript mirror of the core's provider contract (app/core/src/providers/contract.rs). The shared fixtures in
// app/core/tests/contract/ (providers.*.json, project_state.provider.v1.json, providers.invalid.v1.json) are parsed by both
// sides, so the two cannot drift. As in the core: another major reads as "update needed"; unknown fields are ignored; an
// unknown login state reads as "cannot_tell" (never signed in), an unknown tested value as "unknown". Nothing here carries
// a credential: `billing.key_in_app_env` is an environment variable NAME.

import { CONTROL, INVISIBLE, schemaMajor, type ParseResult } from "./projectContract";

export const PROVIDERS_SCHEMA = "vajra.providers/1";
export const PROVIDERS_MAJOR = 1;

export type LoginState = "signed_in" | "signed_out" | "cannot_tell" | "not_installed";
export type Tested = "tested" | "untested" | "unknown";
export type StopKind = "hook_block" | "terminate" | "other";

export interface CapabilityFlags {
  pty_attach: boolean;
  transcript: boolean;
  inspect: boolean;
  message: boolean;
  redirect: boolean;
  stop: StopKind;
  resume: boolean;
  vajra_workflow: boolean;
}

/** A control that is off, with the words to show beside it. Never hidden. */
export interface Limit {
  control: string;
  /** Stable code (`no_message_mid_run`, `stop_ends_process`, `not_installed`, `not_supported`): map it to approved wording; `reason` is the fallback for a code you do not know. */
  code: string | null;
  reason: string;
}

export interface Login {
  route: string;
  route_code: string | null;
  state: LoginState;
  checked_by: string;
  detail: string | null;
  detail_code: string | null;
}

export interface BillingInfo {
  source: string;
  key_in_app_env: string | null;
}

export interface ProviderInfo {
  id: string;
  name: string;
  installed: boolean;
  path: string | null;
  version: string | null;
  tested: Tested;
  tested_range: string | null;
  capabilities: CapabilityFlags;
  limits: Limit[];
  login: Login;
  billing: BillingInfo;
}

export interface ProviderList {
  schema: string;
  providers: ProviderInfo[];
}

/** The additive `provider` object in `project_state`. */
export interface ProjectProvider {
  id: string;
  name: string;
  installed: boolean;
  tested: Tested;
  /** From the last provider_list check; reading project_state never runs a CLI. */
  login_state: LoginState;
  capabilities: CapabilityFlags;
  limits: Limit[];
}

const MAX_BYTES = 64 * 1024;
const MAX_PROVIDERS = 8;
const MAX_LIMITS = 16;
const MAX_TEXT_CHARS = 200;

const whole = (re: RegExp, s: string): boolean => {
  const m = re.exec(s);
  return m !== null && m[0].length === s.length;
};
const isObj = (v: unknown): v is Record<string, unknown> => typeof v === "object" && v !== null && !Array.isArray(v);
const str = (v: unknown): v is string => typeof v === "string";
const optStrOk = (v: unknown): boolean => v === undefined || v === null || str(v);
const optStr = (v: unknown): string | null => (str(v) ? v : null);

type Fail = { ok: false; reason: "unsupported_version" | "malformed" | "invalid"; detail: string };
const bad = (reason: Fail["reason"], detail: string): Fail => ({ ok: false, reason, detail });

const textOk = (s: string): boolean => s.trim() !== "" && [...s].length <= MAX_TEXT_CHARS && !CONTROL.test(s) && !INVISIBLE.test(s);
const validProvider = (p: string): boolean => p !== "" && p.length <= 32 && whole(/[a-z0-9_-]+/, p);
const validLabel = (s: string): boolean => s !== "" && s.length <= 128 && whole(/[A-Za-z0-9_.-]+/, s) && !s.startsWith(".");
const byteLen = (s: string): number => new TextEncoder().encode(s).length;

const LOGIN: readonly LoginState[] = ["signed_in", "signed_out", "not_installed"];
const TESTED: readonly Tested[] = ["tested", "untested"];
const STOPS: readonly StopKind[] = ["hook_block", "terminate"];

/** A closed set where an unknown STRING reads as the catch-all; a non-string is malformed (null here). */
function pick<T extends string>(v: unknown, known: readonly T[], other: T): T | null {
  if (!str(v)) return null;
  return known.find((k) => k === v) ?? other;
}

function capabilities(v: unknown): CapabilityFlags | null {
  if (!isObj(v)) return null;
  const stop = pick(v.stop, STOPS, "other" as StopKind);
  const flags = ["pty_attach", "transcript", "inspect", "message", "redirect", "resume", "vajra_workflow"] as const;
  if (stop === null || flags.some((f) => typeof v[f] !== "boolean")) return null;
  return {
    pty_attach: v.pty_attach as boolean,
    transcript: v.transcript as boolean,
    inspect: v.inspect as boolean,
    message: v.message as boolean,
    redirect: v.redirect as boolean,
    stop,
    resume: v.resume as boolean,
    vajra_workflow: v.vajra_workflow as boolean,
  };
}

function limits(v: unknown): Limit[] | null {
  if (v === undefined || v === null) return [];
  if (!Array.isArray(v)) return null;
  const out: Limit[] = [];
  for (const x of v) {
    if (!isObj(x) || !str(x.control) || !str(x.reason)) return null;
    if (!optStrOk(x.code)) return null;
    out.push({ control: x.control, code: optStr(x.code), reason: x.reason });
  }
  return out;
}

const validCode = (c: string): boolean => c !== "" && c.length <= 64 && whole(/[a-z0-9_]+/, c);
const limitsOk = (l: Limit[]): boolean => l.length <= MAX_LIMITS && l.every((x) => validLabel(x.control) && textOk(x.reason) && (x.code === null || validCode(x.code)));

function parseJson(text: string): { o: Record<string, unknown> } | Fail {
  if (byteLen(text) > MAX_BYTES) return bad("malformed", "answer too large");
  let o: unknown;
  try {
    o = JSON.parse(text);
  } catch {
    return bad("malformed", "not JSON");
  }
  return isObj(o) ? { o } : bad("malformed", "not an object");
}

function provider(o: unknown): ParseResult<ProviderInfo> {
  if (!isObj(o)) return bad("malformed", "provider");
  const lg = o.login;
  const bl = o.billing;
  if (!str(o.id) || !str(o.name) || typeof o.installed !== "boolean" || !optStrOk(o.path) || !optStrOk(o.version) || !optStrOk(o.tested_range)) {
    return bad("malformed", "missing or mistyped field");
  }
  const tested = pick(o.tested, TESTED, "unknown" as Tested);
  const caps = capabilities(o.capabilities);
  const lims = limits(o.limits);
  if (tested === null || caps === null || lims === null || !isObj(lg) || !isObj(bl)) return bad("malformed", "missing or mistyped field");
  const state = pick(lg.state, LOGIN, "cannot_tell" as LoginState);
  if (state === null || !str(lg.route) || !str(lg.checked_by) || !optStrOk(lg.detail) || !optStrOk(lg.route_code) || !optStrOk(lg.detail_code)) return bad("malformed", "login");
  if (!str(bl.source) || !optStrOk(bl.key_in_app_env)) return bad("malformed", "billing");

  if (!validProvider(o.id)) return bad("invalid", "provider id");
  if (!textOk(o.name) || !textOk(lg.route) || !validLabel(lg.checked_by)) return bad("invalid", "text");
  for (const [v, what] of [[o.version, "version"], [o.tested_range, "tested_range"], [lg.detail, "detail"]] as const) {
    if (str(v) && !textOk(v)) return bad("invalid", what);
  }
  for (const [c, what] of [[lg.route_code, "route_code"], [lg.detail_code, "detail_code"]] as const) {
    if (str(c) && !validCode(c)) return bad("invalid", what);
  }
  const path = optStr(o.path);
  if (path !== null && (!path.startsWith("/") || byteLen(path) > 4096 || path.includes("\0"))) return bad("invalid", "path");
  if (o.installed !== (path !== null) || (state === "not_installed") === o.installed) return bad("invalid", "installed");
  if (bl.source !== "login") return bad("invalid", "billing");
  const key = optStr(bl.key_in_app_env);
  if (key !== null && (key === "" || key.length > 64 || !whole(/[A-Z0-9_]+/, key))) return bad("invalid", "billing");
  if (!limitsOk(lims)) return bad("invalid", "limits");
  return {
    ok: true,
    value: {
      id: o.id,
      name: o.name,
      installed: o.installed,
      path,
      version: optStr(o.version),
      tested,
      tested_range: optStr(o.tested_range),
      capabilities: caps,
      limits: lims,
      login: { route: lg.route, route_code: optStr(lg.route_code), state, checked_by: lg.checked_by, detail: optStr(lg.detail), detail_code: optStr(lg.detail_code) },
      billing: { source: bl.source, key_in_app_env: key },
    },
  };
}

/** Mirrors `parse_provider_list`: the tag first (so another major reads as "update needed"), then shape, then rules. */
export function parseProviderList(text: string): ParseResult<ProviderList> {
  const r = parseJson(text);
  if (!("o" in r)) return r;
  const o = r.o;
  if (!str(o.schema)) return bad("malformed", "no schema");
  if (schemaMajor(o.schema, "providers") !== PROVIDERS_MAJOR) return bad("unsupported_version", `${o.schema.slice(0, 64)} (this build reads major ${PROVIDERS_MAJOR})`);
  if (!Array.isArray(o.providers)) return bad("malformed", "providers");
  const out: ProviderInfo[] = [];
  for (const p of o.providers) {
    const x = provider(p);
    if (!x.ok) return x;
    out.push(x.value);
  }
  if (out.length > MAX_PROVIDERS || new Set(out.map((p) => p.id)).size !== out.length) return bad("invalid", "providers");
  return { ok: true, value: { schema: o.schema, providers: out } };
}

/** Mirrors `parse_project_provider`. */
export function parseProjectProvider(text: string): ParseResult<ProjectProvider> {
  const r = parseJson(text);
  if (!("o" in r)) return r;
  const o = r.o;
  const caps = capabilities(o.capabilities);
  const lims = limits(o.limits);
  const state = pick(o.login_state, LOGIN, "cannot_tell" as LoginState);
  const tested = pick(o.tested, TESTED, "unknown" as Tested);
  if (!str(o.id) || !str(o.name) || typeof o.installed !== "boolean" || tested === null || state === null || caps === null || lims === null) return bad("malformed", "missing or mistyped field");
  if (!textOk(o.name) || !validProvider(o.id) || (state === "not_installed") === o.installed) return bad("invalid", "provider");
  if (!limitsOk(lims)) return bad("invalid", "limits");
  return { ok: true, value: { id: o.id, name: o.name, installed: o.installed, tested, login_state: state, capabilities: caps, limits: lims } };
}

// ---- `vajra.provider_accounts/1` (types only; the parser arrives with the Providers screen) ----------------------------
// One record per way of running and paying for agents. There is no field that can hold a key, token or secret, and no
// rotation field (H7): a stored key is only the boolean `credential.saved`. Unknown states read as "cannot_tell".

export const ACCOUNTS_SCHEMA = "vajra.provider_accounts/1";

export type CredentialSource = "cli_login" | "api_key" | "gateway";
export type BillingKind = "subscription" | "api_key" | "gateway" | "unknown";
export type AccountState = "signed_in" | "signed_out" | "not_installed" | "key_saved" | "key_missing" | "cannot_tell";

export interface AccountCredential {
  source: CredentialSource;
  /** `api_key` and `gateway` only: whether a key is stored. Never the key, a prefix or a suffix. */
  saved?: boolean;
}

/** An API key setting that would bill instead of the subscription: the NAME of the setting and where it was found. */
export interface OverrideWarning {
  name: string;
  found_in: "cli_env_block" | "cli_config" | "cli_api_key_helper";
}

export interface AccountBilling {
  kind: BillingKind;
  /** Only what the CLI itself prints. */
  account_label: string | null;
  override_warning: OverrideWarning | null;
}

export interface AccountModel {
  /** Opaque; passed straight through to the harness. */
  id: string;
  /** Picks the icon. */
  family: string;
}

export interface Account {
  id: string;
  provider: string;
  /** The unmodified CLI that runs the worker. */
  harness: string;
  credential: AccountCredential;
  billing: AccountBilling;
  label: string;
  state: AccountState;
  models: AccountModel[];
}

export interface AccountList {
  schema: string;
  accounts: Account[];
}

/** An account with the model and effort to ask it for; `null` is the CLI's own default. */
export interface AgentChoice {
  account: string;
  model: string | null;
  effort: string | null;
}

/** One ceiling entry; a `model` of `null` allows every model on that account. */
export interface AllowedAgent {
  account: string;
  model: string | null;
}

/** Per-project agent choice (`preferences.agents`). A role with no entry uses `default`. The old `agent` preference reads as
 *  `{default: {account: agent}, roles: {}, allowed: [{account: agent}]}`. */
export interface Agents {
  default: AgentChoice;
  roles: Record<string, AgentChoice>;
  allowed: AllowedAgent[];
}

// ---- `vajra.provider_usage/1` (H5): what an account has used, as its own source reports it ------------------------------
// `windows` is empty when nothing is reported: say "Not reported", never estimate. `warn` is set at 80% used.

export const USAGE_SCHEMA = "vajra.provider_usage/1";

export interface UsageWindow {
  /** `5h`, `daily`, `weekly` or `credit` (a key's own spending limit). */
  kind: string;
  used_pct: number;
  resets_at: string | null;
  warn: boolean;
}

export interface Usage {
  account: string;
  read_at: string | null;
  /** `none` or `gateway_key`. */
  source: string;
  windows: UsageWindow[];
  amounts: { usage: number; usage_daily: number | null; limit: number | null; limit_remaining: number | null } | null;
}

export interface UsageList {
  schema: string;
  usage: Usage[];
}

/** Accept a `provider_usage` answer only if it has the shape the chip reads; anything else is "not reported" (null). */
export function parseUsage(v: unknown): UsageList | null {
  if (!isObj(v) || v.schema !== USAGE_SCHEMA || !Array.isArray(v.usage)) return null;
  const windowOk = (w: unknown): w is UsageWindow => isObj(w) && str(w.kind) && typeof w.used_pct === "number" && Number.isFinite(w.used_pct)
    && typeof w.warn === "boolean" && (w.resets_at === null || w.resets_at === undefined || str(w.resets_at));
  const entryOk = (u: unknown): u is Usage => isObj(u) && str(u.account) && validProvider(u.account) && Array.isArray(u.windows) && u.windows.every(windowOk);
  if (v.usage.length > MAX_PROVIDERS * 2 || !v.usage.every(entryOk)) return null;
  return { schema: USAGE_SCHEMA, usage: v.usage.map((u) => ({ ...u, windows: u.windows.map((w) => ({ ...w, resets_at: w.resets_at ?? null })) })) };
}

/** Account ids the app ships with, by the label the rest of the UI shows for them. The usage answer carries only the id and the idle shell must not load the accounts list just to name one. */
const ACCOUNT_LABELS: Record<string, string> = { claude: "Claude Code", codex: "Codex CLI", openrouter: "OpenRouter" };
/** What to call an account in a warning: its label, or the id when no label is known. */
export const accountLabel = (id: string): string => ACCOUNT_LABELS[id] ?? id;

/** The window the status bar chip is about: the most used one that is at or over the warning level. */
export function usageWarning(list: UsageList | null | undefined): { account: string; window: UsageWindow } | null {
  let worst: { account: string; window: UsageWindow } | null = null;
  for (const u of list?.usage ?? []) {
    for (const w of u.windows) {
      if (w.warn && (worst === null || w.used_pct > worst.window.used_pct)) worst = { account: u.account, window: w };
    }
  }
  return worst;
}

// ---- launch receipt (H3): what a slot asked for next to what the harness said it ran on ---------------------------------
// A value the harness did not report is the text "unknown" ("Not reported"); it is never the requested one.

export const UNKNOWN = "unknown";

export interface LaunchRequested {
  account: string;
  harness: string;
  model: string;
  effort: string | null;
}

export interface LaunchEffective extends LaunchRequested {
  sandbox: string;
  billing: string;
  /** `harness_session_file`, `harness_event`, or `null` when nothing was read. */
  read_from: string | null;
}

export interface LaunchReceipt {
  slot: string;
  session_id: string;
  source: string;
  requested: LaunchRequested;
  effective: LaunchEffective;
}

/** The fields that differ between what was asked for and what ran; a value reported as "unknown" never counts. */
export function launchMismatches(r: Pick<LaunchReceipt, "requested" | "effective">): string[] {
  const { requested: q, effective: e } = r;
  const out: string[] = [];
  if (e.model !== UNKNOWN && e.model !== q.model) out.push("model");
  if (q.effort !== null && e.effort !== null && e.effort !== UNKNOWN && e.effort !== q.effort) out.push("effort");
  if (e.account !== UNKNOWN && e.account !== q.account) out.push("account");
  return out;
}
