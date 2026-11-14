// The UI's reading of the multi-provider data (`vajra.provider_accounts/1`, `vajra.provider_usage/1`, the project `agents`
// block). The core side lands in later PRs; until then these shapes come from devFixture.ts and the tests. Nothing here
// carries a credential: an api_key account holds `saved: true|false` and nothing else, and every parser keeps only the
// fields named below, so a stray secret in an answer cannot reach a component.
import { CONTROL, INVISIBLE, schemaMajor } from "./projectContract";
import { S } from "../strings";
import type { ParseResult } from "./projectContract";

export const ACCOUNTS_SCHEMA = "vajra.provider_accounts/1";
export const USAGE_SCHEMA = "vajra.provider_usage/1";
export const AGENTS_SCHEMA = "vajra.project_agents/1";

export type AccountState = "signed_in" | "signed_out" | "cannot_tell" | "not_installed" | "key_saved" | "key_missing";
export type BillingKind = "subscription" | "api_key" | "gateway" | "unknown";
export type ToolCheck = "passed" | "unchecked" | "failed";

export interface AccountModel {
  id: string;
  family: string;
  /** A display name when the core knows one; otherwise the id is shown, still with its provider. */
  name: string | null;
  tool_check: ToolCheck;
  /** For a "failed" check: the date, as the core wrote it. */
  checked_on: string | null;
}

export interface Account {
  id: string;
  provider: string;
  harness: string;
  label: string;
  state: AccountState;
  credential: { source: "cli_login" | "api_key" | "gateway"; saved: boolean | null };
  billing: { kind: BillingKind; account_label: string | null; override_warning: OverrideWarning | null };
  /** The efforts this account's harness takes. Empty: it has no effort setting (the receipt says "Not reported", never a guess). */
  efforts: string[];
  /** A gateway's non-secret base URL. */
  base_url: string | null;
  models: AccountModel[];
}

export interface AccountList { schema: string; accounts: Account[] }

export interface UsageWindow {
  kind: "5h" | "daily" | "weekly" | "credit";
  used_pct: number | null;
  resets_at: string | null;
  /** Credit windows only (a key's own `usage` and `limit`, as the gateway reports them). */
  used_usd: number | null;
  limit_usd: number | null;
}
export interface Usage { account: string; read_at: string | null; windows: UsageWindow[] }

export interface AgentPick { account: string; model: string | null; effort: string | null }
export interface Allowed { account: string; model: string | null }
export interface Agents { default: AgentPick; roles: Record<string, AgentPick>; allowed: Allowed[] }

/** The keys the core accepts for a per-role default: its seat roles (`crew::roles`) plus the planner's `architect`. Keep the two lists the same. */
export const ROLES = ["architect", "backend", "frontend", "tests", "verifier", "docs", "reviewer", "security_reviewer", "platform", "numerics", "analysis"] as const;

/** OpenRouter models carry "New" until the soak passes (build brief, PR 8). One switch, so the label comes off in one edit. */
export const GATEWAY_SOAK_PASSED = false;
/** The Advanced > base URL box is shown only while the core can save one. The core has no `provider_base_url_save` command yet,
 *  so the box would be a control that always errors; flip this when the command lands. */
export const BASE_URL_SAVE_WIRED = false;
export const WARN_PCT = 80;

/** The core's `override_warning`: the NAME of the setting that would bill an API key, and where it was found. Never a value. */
export interface OverrideWarning { name: string; found_in: "cli_env_block" | "cli_config" | "cli_api_key_helper" }
const FOUND_IN = ["cli_env_block", "cli_config", "cli_api_key_helper"] as const;

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === "object" && v !== null && !Array.isArray(v);
const str = (v: unknown): v is string => typeof v === "string";
const optStr = (v: unknown): string | null => (str(v) && v !== "" ? v : null);
const optNum = (v: unknown): number | null => (typeof v === "number" && Number.isFinite(v) ? v : null);
const bad = (reason: "malformed" | "invalid", detail: string) => ({ ok: false as const, reason, detail });
const label = (s: string): boolean => s.trim() !== "" && [...s].length <= 200 && !CONTROL.test(s) && !INVISIBLE.test(s);
const idOk = (s: string): boolean => /^[A-Za-z0-9_./:-]{1,128}$/.test(s);
const known = <T extends string>(v: unknown, set: readonly T[], other: T): T => set.find((k) => k === v) ?? other;

const STATES = ["signed_in", "signed_out", "cannot_tell", "not_installed", "key_saved", "key_missing"] as const;
const KINDS = ["subscription", "api_key", "gateway", "unknown"] as const;
const CHECKS = ["passed", "unchecked", "failed"] as const;
const SOURCES = ["cli_login", "api_key", "gateway"] as const;
const WINDOWS = ["5h", "daily", "weekly", "credit"] as const;

function model(v: unknown): AccountModel | null {
  if (!isObj(v) || !str(v.id) || !idOk(v.id) || !str(v.family) || !idOk(v.family)) return null;
  const name = optStr(v.name);
  if (name !== null && !label(name)) return null;
  return { id: v.id, family: v.family, name, tool_check: known(v.tool_check, CHECKS, "unchecked"), checked_on: optStr(v.checked_on) };
}

/** `null` when there is none; `"bad"` when something is there that is not the core's `{name, found_in}` (the account then fails). */
function overrideWarning(v: unknown): OverrideWarning | null | "bad" {
  if (v === null || v === undefined) return null;
  if (!isObj(v) || !str(v.name) || !/^[A-Za-z0-9_]{1,64}$/.test(v.name)) return "bad";
  return { name: v.name, found_in: known(v.found_in, FOUND_IN, "cli_config") };
}

function account(v: unknown): Account | null {
  if (!isObj(v) || !str(v.id) || !idOk(v.id) || !str(v.provider) || !idOk(v.provider) || !str(v.harness) || !str(v.label) || !label(v.label)) return null;
  const cred = isObj(v.credential) ? v.credential : {};
  const bill = isObj(v.billing) ? v.billing : {};
  const models: AccountModel[] = [];
  for (const m of Array.isArray(v.models) ? v.models : []) {
    const x = model(m);
    if (!x) return null;
    models.push(x);
  }
  const warning = overrideWarning(bill.override_warning);
  const acct = optStr(bill.account_label);
  if (warning === "bad" || (acct !== null && !label(acct))) return null;
  return {
    id: v.id, provider: v.provider, harness: v.harness, label: v.label,
    state: known(v.state, STATES, "cannot_tell"),
    credential: { source: known(cred.source, SOURCES, "cli_login"), saved: typeof cred.saved === "boolean" ? cred.saved : null },
    billing: { kind: known(bill.kind, KINDS, "unknown"), account_label: acct, override_warning: warning },
    efforts: Array.isArray(v.efforts) ? v.efforts.filter((e): e is string => str(e) && idOk(e)) : [],
    base_url: optStr(v.base_url),
    models,
  };
}

/** The tag first (another major reads as "update needed"), then shape. A model or account that fails fails the whole answer. */
export function parseAccounts(v: unknown): ParseResult<AccountList> {
  if (!isObj(v) || !str(v.schema)) return bad("malformed", "no schema");
  if (schemaMajor(v.schema, "provider_accounts") !== 1) return { ok: false, reason: "unsupported_version", detail: v.schema.slice(0, 64) };
  if (!Array.isArray(v.accounts)) return bad("malformed", "accounts");
  const accounts: Account[] = [];
  for (const a of v.accounts) {
    const x = account(a);
    if (!x) return bad("invalid", "account");
    accounts.push(x);
  }
  if (new Set(accounts.map((a) => a.id)).size !== accounts.length) return bad("invalid", "accounts");
  return { ok: true, value: { schema: v.schema, accounts } };
}

export function parseUsage(v: unknown): ParseResult<Usage[]> {
  if (!isObj(v) || !str(v.schema)) return bad("malformed", "no schema");
  if (schemaMajor(v.schema, "provider_usage") !== 1) return { ok: false, reason: "unsupported_version", detail: v.schema.slice(0, 64) };
  if (!Array.isArray(v.usage)) return bad("malformed", "usage");
  const out: Usage[] = [];
  for (const u of v.usage) {
    if (!isObj(u) || !str(u.account) || !Array.isArray(u.windows)) return bad("malformed", "usage");
    const windows: UsageWindow[] = [];
    for (const w of u.windows) {
      if (!isObj(w) || !WINDOWS.includes(w.kind as never)) continue; // an unknown window kind is skipped, not guessed at
      windows.push({ kind: w.kind as UsageWindow["kind"], used_pct: optNum(w.used_pct), resets_at: optStr(w.resets_at), used_usd: optNum(w.used_usd), limit_usd: optNum(w.limit_usd) });
    }
    out.push({ account: u.account, read_at: optStr(u.read_at), windows });
  }
  return { ok: true, value: out };
}

const pickOf = (v: unknown): AgentPick | null => {
  if (!isObj(v) || !str(v.account) || !idOk(v.account)) return null;
  return { account: v.account, model: optStr(v.model), effort: optStr(v.effort) };
};

export function parseAgents(v: unknown): ParseResult<Agents> {
  if (!isObj(v) || !str(v.schema)) return bad("malformed", "no schema");
  if (schemaMajor(v.schema, "project_agents") !== 1) return { ok: false, reason: "unsupported_version", detail: v.schema.slice(0, 64) };
  const def = pickOf(v.default);
  if (!def || !isObj(v.roles) || !Array.isArray(v.allowed)) return bad("malformed", "agents");
  const roles: Record<string, AgentPick> = {};
  for (const [k, r] of Object.entries(v.roles)) {
    const p = pickOf(r);
    if (!p || !(ROLES as readonly string[]).includes(k)) return bad("invalid", "role");
    roles[k] = p;
  }
  const allowed: Allowed[] = [];
  for (const a of v.allowed) {
    if (!isObj(a) || !str(a.account) || !idOk(a.account)) return bad("invalid", "allowed");
    allowed.push({ account: a.account, model: optStr(a.model) });
  }
  return { ok: true, value: { default: def, roles, allowed } };
}

/** What a model is called on screen: its name, or its id when the core gave none. */
export const modelName = (m: AccountModel): string => m.name ?? m.id;

/** H6: a model row always names its provider. `model: null` is the CLI's own default model for that login. A bare model id is a bug. */
export function modelLine(a: Account, modelId: string | null, defaultWord: string): string {
  const m = modelId ? a.models.find((x) => x.id === modelId) : undefined;
  return S.accounts.modelVia(modelId ? (m ? modelName(m) : modelId) : defaultWord, a.label);
}

export const isNewModel = (a: Account): boolean => a.provider === "openrouter" && !GATEWAY_SOAK_PASSED;

/** The 80% warning: the highest reported window at or past the line, or null. A window that reports nothing never warns. */
export function warnWindow(u: Usage | undefined): UsageWindow | null {
  const hot = (u?.windows ?? []).filter((w) => pct(w) !== null && (pct(w) as number) >= WARN_PCT);
  return hot.sort((x, y) => (pct(y) as number) - (pct(x) as number))[0] ?? null;
}

export function pct(w: UsageWindow): number | null {
  if (w.used_pct !== null) return Math.max(0, Math.min(100, Math.round(w.used_pct)));
  if (w.used_usd !== null && w.limit_usd !== null && w.limit_usd > 0) return Math.max(0, Math.min(100, Math.round((w.used_usd / w.limit_usd) * 100)));
  return null;
}

/** True when an allowance for the whole account (`model: null`) covers this entry. The core reads `{account, model: null}` as every model on that account. */
export const coveredByAccount = (allowed: Allowed[], account: string): boolean => allowed.some((x) => x.account === account && x.model === null);

/**
 * What one tick in the allowed list does. Under an account-wide allowance (`model: null`) ANY row on that account (a model, or a CLI
 * login's default, which is that entry) turns it into explicit entries for every model except the one switched off, so one click never
 * flips another row. Otherwise it adds or removes just that entry; adding an account-wide entry drops that account's now-redundant model entries.
 */
export function toggleAllowed(allowed: Allowed[], e: Allowed, models: string[]): Allowed[] {
  const same = (a: Allowed, b: Allowed) => a.account === b.account && a.model === b.model;
  if (coveredByAccount(allowed, e.account)) {
    const rest = models.filter((id) => id !== e.model).map((id) => ({ account: e.account, model: id }));
    return [...allowed.filter((x) => !(x.account === e.account && x.model === null)), ...rest.filter((r) => !allowed.some((x) => same(x, r)))];
  }
  if (allowed.some((x) => same(x, e))) return allowed.filter((x) => !same(x, e));
  return e.model === null ? [...allowed.filter((x) => x.account !== e.account), e] : [...allowed, e];
}

/** Every choice a project may use, in the accounts' order. An account allowed at `null` offers its own default (a CLI login only) and all of its models; a concrete allowance offers just that model. */
export function allowedChoices(accounts: Account[], allowed: Allowed[]): Array<{ account: Account; model: string | null }> {
  const out: Array<{ account: Account; model: string | null }> = [];
  for (const a of accounts) {
    const all = coveredByAccount(allowed, a.id);
    if (all && a.credential.source === "cli_login") out.push({ account: a, model: null });
    for (const m of a.models) if (all || allowed.some((x) => x.account === a.id && x.model === m.id)) out.push({ account: a, model: m.id });
  }
  return out;
}

export const choiceKey = (account: string, model: string | null): string => `${account}|${model ?? ""}`;

const LOOPBACK = new Set(["localhost", "127.0.0.1", "[::1]"]);

/** A gateway base URL is non-secret: https (http only to this machine, since the gateway key is sent to it), a host, and no user:password in it (that would be a credential stored in the clear). */
export function baseUrlProblem(raw: string): "empty" | "scheme" | "insecure" | "credentials" | null {
  const s = raw.trim();
  if (s === "") return "empty";
  let u: URL;
  try { u = new URL(s); } catch { return "scheme"; }
  if (u.protocol !== "https:" && u.protocol !== "http:") return "scheme";
  if (u.protocol === "http:" && !LOOPBACK.has(u.hostname)) return "insecure";
  if (u.username || u.password) return "credentials";
  return null;
}

/** One worker of a run proposal (spec section 3). `source` says where the pick came from; `why` is the Director-General's own words (plain text, untrusted). */
export interface ProposalWorker { slot: string; role: string; task: string; why: string; source: "role_default" | "dg" | "user"; pick: AgentPick }
/** What Start carries for a pick the user changed: validated again by the core, and recorded there as source "user". */
export interface ProposalOverride { seat: string; account?: string; model?: string | null; effort?: string | null; network?: boolean }

export function parseProposalWorkers(v: unknown): ProposalWorker[] {
  if (!Array.isArray(v)) return [];
  const out: ProposalWorker[] = [];
  for (const w of v.slice(0, 16)) {
    const p = isObj(w) ? pickOf(w) : null;
    if (!isObj(w) || !p || !str(w.slot) || !idOk(w.slot) || !str(w.role) || !str(w.task) || !label(w.task)) continue;
    out.push({ slot: w.slot, role: w.role, task: w.task, why: str(w.why) && label(w.why) ? w.why : "", source: known(w.source, ["role_default", "dg", "user"] as const, "role_default"), pick: p });
  }
  return out;
}
