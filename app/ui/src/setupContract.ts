// The UI's reading of `setup_list` (`vajra.harness_setup/1`, app/core/src/providers/setup.rs): Settings > Providers >
// Set up. Another major reads as "update needed". An unknown state reads as "manual" (install it yourself), never as
// installed. Only the fields named here are kept, and a docs link must be https, so nothing else reaches a component.
import { CONTROL, INVISIBLE, schemaMajor } from "./projectContract";
import type { ParseResult } from "./projectContract";

export const SETUP_SCHEMA = "vajra.harness_setup/1";
const SETUP_MAJOR = 1;

export type SetupState = "installed" | "unsupported" | "broken" | "manual";
const STATES: readonly SetupState[] = ["installed", "unsupported", "broken", "manual"];

export interface SetupCommand { tool: string; command: string }

export interface SetupRow {
  id: string;
  name: string;
  state: SetupState;
  version: string | null;
  path: string | null;
  reason_code: string | null;
  /** The vendor's own commands, for the user to run themselves. */
  commands: SetupCommand[];
  docs_url: string;
  can_sign_in: boolean;
  /** A verified catalog source this Mac may install or repair in one click (`setup_plan`). */
  one_click: boolean;
}

export interface SetupList { rows: SetupRow[]; engine: { installed: boolean; command: string } }

type O = Record<string, unknown>;
const obj = (v: unknown): v is O => typeof v === "object" && v !== null && !Array.isArray(v);
const text = (v: unknown, max = 300): v is string => typeof v === "string" && v.length > 0 && v.length <= max && !CONTROL.test(v) && !INVISIBLE.test(v);
const optText = (v: unknown, max = 300): string | null | undefined => (v === null || v === undefined ? null : text(v, max) ? v : undefined);
const https = (v: unknown): v is string => text(v, 2048) && /^https:\/\/[^/\s]/.test(v);

function row(v: unknown): SetupRow | null {
  if (!obj(v) || !text(v.id, 64) || !text(v.name, 120) || !https(v.docs_url) || !Array.isArray(v.commands)) return null;
  const version = optText(v.version, 64);
  const path = optText(v.path, 4096);
  const reason = optText(v.reason_code, 64);
  if (version === undefined || path === undefined || reason === undefined) return null;
  const commands: SetupCommand[] = [];
  for (const c of v.commands.slice(0, 8)) {
    if (!obj(c) || !text(c.tool, 16) || !text(c.command)) return null;
    commands.push({ tool: c.tool, command: c.command });
  }
  const known = STATES.includes(v.state as SetupState);
  const state = known ? (v.state as SetupState) : "manual";
  return { id: v.id, name: v.name, state, version, path, reason_code: reason, commands, docs_url: v.docs_url, can_sign_in: known && v.can_sign_in === true, one_click: known && v.one_click === true };
}

export function parseSetupList(v: unknown): ParseResult<SetupList> {
  if (!obj(v) || typeof v.schema !== "string") return { ok: false, reason: "malformed", detail: "not a setup list" };
  if (schemaMajor(v.schema, "harness_setup") !== SETUP_MAJOR) return { ok: false, reason: "unsupported_version", detail: v.schema };
  if (!Array.isArray(v.rows) || !obj(v.engine) || typeof v.engine.installed !== "boolean" || !text(v.engine.command)) return { ok: false, reason: "invalid", detail: "rows or engine" };
  const rows: SetupRow[] = [];
  for (const r of v.rows.slice(0, 32)) {
    const p = row(r);
    if (!p) return { ok: false, reason: "invalid", detail: "row" };
    rows.push(p);
  }
  return { ok: true, value: { rows, engine: { installed: v.engine.installed, command: v.engine.command } } };
}

// `setup_plan` (`vajra.setup_plan/1`): the one-click plan, or why it is install it yourself. Only these fields are kept.
export const SETUP_PLAN_SCHEMA = "vajra.setup_plan/1";

export interface SetupTool { role: string; path: string; label: string | null; owned_by_you: boolean }
export interface SetupPlan {
  id: string;
  method: string;
  command: string[];
  tools: SetupTool[];
  source: string;
  version: string;
  target: string;
  needs_admin: { path: string; reason: string } | null;
  notes: string[];
  scope_registry: string | null;
  deps: string[];
  hash: string;
}
export type PlanAnswer = { kind: "plan"; plan: SetupPlan } | { kind: "manual"; code: string };

const texts = (v: unknown, n: number, max = 4096): string[] | null =>
  Array.isArray(v) && v.length <= n && v.every((x) => text(x, max)) ? (v as string[]) : null;

function plan(v: unknown): SetupPlan | null {
  if (!obj(v) || !text(v.id, 64) || !text(v.method, 32) || typeof v.hash !== "string" || !/^[0-9a-f]{64}$/.test(v.hash)) return null;
  const command = texts(v.command, 32);
  const notes = texts(v.notes, 16, 64);
  const deps = Array.isArray(v.deps) && v.deps.length === 0 ? [] : texts(v.deps, 64, 200);
  if (!command || command.length === 0 || !notes || !deps || !Array.isArray(v.tools) || v.tools.length > 8) return null;
  const source = optText(v.source, 300);
  const version = v.version === "" ? "" : optText(v.version, 64);
  const target = v.target === "" ? "" : optText(v.target, 4096);
  const scope = optText(v.scope_registry, 2048);
  if (source === undefined || version === undefined || target === undefined || scope === undefined) return null;
  const tools: SetupTool[] = [];
  for (const t of v.tools) {
    if (!obj(t) || !text(t.role, 16) || !text(t.path, 4096)) return null;
    const label = optText(t.label, 32);
    if (label === undefined) return null;
    tools.push({ role: t.role, path: t.path, label, owned_by_you: t.owned_by_you === true });
  }
  let admin: SetupPlan["needs_admin"] = null;
  if (v.needs_admin !== null && v.needs_admin !== undefined) {
    if (!obj(v.needs_admin) || !text(v.needs_admin.path, 4096) || !text(v.needs_admin.reason, 32)) return null;
    admin = { path: v.needs_admin.path, reason: v.needs_admin.reason };
  }
  return { id: v.id, method: v.method, command, tools, source: source ?? "", version: version ?? "", target: target ?? "", needs_admin: admin, notes, scope_registry: scope, deps, hash: v.hash };
}

export function parseSetupPlan(v: unknown): ParseResult<PlanAnswer> {
  if (!obj(v) || typeof v.schema !== "string") return { ok: false, reason: "malformed", detail: "not a setup plan" };
  if (schemaMajor(v.schema, "setup_plan") !== 1) return { ok: false, reason: "unsupported_version", detail: v.schema };
  if (text(v.manual, 64)) return { ok: true, value: { kind: "manual", code: v.manual } };
  const p = plan(v.plan);
  return p ? { ok: true, value: { kind: "plan", plan: p } } : { ok: false, reason: "invalid", detail: "plan" };
}

export type RunOutcome = "installed" | "failed" | "rolled_back" | "broken";
const OUTCOMES: readonly RunOutcome[] = ["installed", "failed", "rolled_back", "broken"];
export interface RunReport { outcome: RunOutcome; reason: string | null; exit: number | null; version: string | null }

/** `setup_run`: an unknown outcome reads as broken, never as installed. */
export function parseSetupRun(v: unknown): RunReport | null {
  if (!obj(v) || !obj(v.report)) return null;
  const r = v.report;
  const reason = optText(r.reason, 64);
  const version = optText(r.version, 64);
  if (reason === undefined || version === undefined) return null;
  const outcome = OUTCOMES.includes(r.outcome as RunOutcome) ? (r.outcome as RunOutcome) : "broken";
  return { outcome, reason, exit: typeof r.exit === "number" ? r.exit : null, version };
}
