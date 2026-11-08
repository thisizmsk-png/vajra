// The TypeScript mirror of the core's project and session contract (app/core/src/projects/contract.rs). The shared JSON
// fixtures in app/core/tests/contract/ are parsed by both sides (tests/projectContract.test.ts here, tests/contract.rs
// there), so the two cannot drift. Rules, as in the core: every record carries `schema: "vajra.<thing>/<major>"`; a
// record of another major is refused as "update needed"; unknown fields are ignored; an unknown kind or status reads as
// the catch-all, never as success.

export const PROJECT_SCHEMA = "vajra.project/1";
export const SESSION_SCHEMA = "vajra.session/1";
export const PROJECT_MAJOR = 1;
export const SESSION_MAJOR = 1;

export interface RootKey {
  dev: number;
  ino: number;
  git_common_dir: string | null;
}

export interface Project {
  schema: string;
  id: string;
  name: string;
  root: string;
  root_key: RootKey;
  created: string;
  archived: string | null;
  rev: number;
}

export type SessionKind = "orchestrator" | "run" | "worker" | "terminal_note" | "other";
export type SessionStatus = "running" | "stopped" | "done" | "failed" | "unknown";

export interface Session {
  schema: string;
  id: string;
  project_id: string;
  kind: SessionKind;
  parent: string | null;
  provider: string;
  native_session_id: string | null;
  run_id: string | null;
  campaign: string | null;
  task_id: string | null;
  attempt: number | null;
  generation: number;
  status: SessionStatus;
  started: string;
  ended: string | null;
}

/** What `app_info` returns: the contract majors the running core supports. */
export interface AppInfo {
  schema: string;
  contracts: { project: number; session: number };
}

export type ParseResult<T> = { ok: true; value: T } | { ok: false; reason: "unsupported_version" | "malformed" | "invalid"; detail: string };

const CROCKFORD = /^[0-7][0-9A-HJKMNP-TV-Z]{25}$/;

/** `<prefix>_` and 26 Crockford base32 characters, the first at most 7. */
export function validId(prefix: "p" | "s" | "a", id: unknown): id is string {
  return typeof id === "string" && id.startsWith(`${prefix}_`) && CROCKFORD.test(id.slice(2));
}

/** The major of a `vajra.<thing>/<major>` tag, or null for any other shape. */
export function schemaMajor(tag: unknown, thing: string): number | null {
  if (typeof tag !== "string") return null;
  const m = new RegExp(`^vajra\\.${thing}/(0|[1-9][0-9]{0,3})$`).exec(tag);
  return m ? Number(m[1]) : null;
}

const KINDS: readonly SessionKind[] = ["orchestrator", "run", "worker", "terminal_note"];
const STATUSES: readonly SessionStatus[] = ["running", "stopped", "done", "failed"];

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === "object" && v !== null && !Array.isArray(v);
const str = (v: unknown): v is string => typeof v === "string";
const optStr = (v: unknown): string | null => (str(v) ? v : null);
const bad = <T>(reason: "malformed" | "invalid" | "unsupported_version", detail: string): ParseResult<T> => ({ ok: false, reason, detail });

function checkTag(o: Record<string, unknown>, thing: string, supported: number): ParseResult<never> | null {
  if (schemaMajor(o.schema, thing) !== supported) return bad("unsupported_version", `${String(o.schema).slice(0, 64)} (this build reads major ${supported})`);
  return null;
}

export function parseProject(text: string): ParseResult<Project> {
  let o: unknown;
  try {
    o = JSON.parse(text);
  } catch {
    return bad("malformed", "not JSON");
  }
  if (!isObj(o) || !("schema" in o)) return bad("malformed", "not a record");
  const tag = checkTag(o, "project", PROJECT_MAJOR);
  if (tag) return tag;
  const rk = o.root_key;
  if (!validId("p", o.id)) return bad("invalid", "project id");
  if (!str(o.name) || !str(o.root) || !str(o.created) || typeof o.rev !== "number" || !isObj(rk) || typeof rk.dev !== "number" || typeof rk.ino !== "number") {
    return bad("malformed", "missing or mistyped field");
  }
  if (!o.root.startsWith("/")) return bad("invalid", "root");
  return {
    ok: true,
    value: {
      schema: o.schema as string,
      id: o.id,
      name: o.name,
      root: o.root,
      root_key: { dev: rk.dev, ino: rk.ino, git_common_dir: optStr(rk.git_common_dir) },
      created: o.created,
      archived: optStr(o.archived),
      rev: o.rev,
    },
  };
}

export function parseSession(text: string): ParseResult<Session> {
  let o: unknown;
  try {
    o = JSON.parse(text);
  } catch {
    return bad("malformed", "not JSON");
  }
  if (!isObj(o) || !("schema" in o)) return bad("malformed", "not a record");
  const tag = checkTag(o, "session", SESSION_MAJOR);
  if (tag) return tag;
  if (!validId("s", o.id) || !validId("p", o.project_id)) return bad("invalid", "id");
  if (!str(o.provider) || !str(o.started) || typeof o.generation !== "number") return bad("malformed", "missing or mistyped field");
  const kind = KINDS.find((k) => k === o.kind) ?? "other";
  const status = STATUSES.find((s) => s === o.status) ?? "unknown";
  return {
    ok: true,
    value: {
      schema: o.schema as string,
      id: o.id,
      project_id: o.project_id,
      kind,
      parent: optStr(o.parent),
      provider: o.provider,
      native_session_id: optStr(o.native_session_id),
      run_id: optStr(o.run_id),
      campaign: optStr(o.campaign),
      task_id: optStr(o.task_id),
      attempt: typeof o.attempt === "number" ? o.attempt : null,
      generation: o.generation,
      status,
      started: o.started,
      ended: optStr(o.ended),
    },
  };
}

/** True when the running core speaks the contract majors this UI was built for; otherwise show "update needed". */
export function supportsContracts(info: AppInfo): boolean {
  return info.contracts.project === PROJECT_MAJOR && info.contracts.session === SESSION_MAJOR;
}
