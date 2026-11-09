// The UI's reading of the multi-provider data (`vajra.provider_accounts/1`, `vajra.provider_usage/1`, the project `agents`
// block). The core side lands in later PRs; until then these shapes come from devFixture.ts and the tests. Nothing here
// carries a credential: an api_key account holds `saved: true|false` and nothing else, and every parser keeps only the
// fields named below, so a stray secret in an answer cannot reach a component.
import { CONTROL, INVISIBLE, schemaMajor } from "./projectContract";
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
  billing: { kind: BillingKind; account_label: string | null; override_warning: string | null };
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

/** The crew roles the workflow already uses; a per-role default is keyed by one of these. */
export const ROLES = ["architect", "backend", "frontend", "tests", "verifier", "docs"] as const;

/** OpenRouter models carry "New" until the soak passes (build brief, PR 8). One switch, so the label comes off in one edit. */
export const GATEWAY_SOAK_PASSED = false;
export const WARN_PCT = 80;

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
  const warning = optStr(bill.override_warning);
  const acct = optStr(bill.account_label);
  if ((warning !== null && !label(warning)) || (acct !== null && !label(acct))) return null;
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
  return `${modelId ? (m ? modelName(m) : modelId) : defaultWord} · ${a.label}`;
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

/** Every choice a project may use: one per allowed account default, one per allowed model. Same order as the accounts. */
export function allowedChoices(accounts: Account[], allowed: Allowed[]): Array<{ account: Account; model: string | null }> {
  const out: Array<{ account: Account; model: string | null }> = [];
  for (const a of accounts) {
    for (const al of allowed) if (al.account === a.id && (al.model === null || a.models.some((m) => m.id === al.model))) out.push({ account: a, model: al.model });
  }
  return out;
}

export const choiceKey = (account: string, model: string | null): string => `${account}|${model ?? ""}`;

/** A gateway base URL is non-secret: http(s), a host, and no user:password in it (that would be a credential stored in the clear). */
export function baseUrlProblem(raw: string): "empty" | "scheme" | "credentials" | null {
  const s = raw.trim();
  if (s === "") return "empty";
  let u: URL;
  try { u = new URL(s); } catch { return "scheme"; }
  if (u.protocol !== "https:" && u.protocol !== "http:") return "scheme";
  if (u.username || u.password) return "credentials";
  return null;
}
