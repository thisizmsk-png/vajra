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
  return { id: v.id, name: v.name, state, version, path, reason_code: reason, commands, docs_url: v.docs_url, can_sign_in: known && v.can_sign_in === true };
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
