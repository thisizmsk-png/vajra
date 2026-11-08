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

export type ParseResult<T> = { ok: true; value: T } | { ok: false; reason: "unsupported_version" | "malformed" | "invalid"; detail: string };

/** What `app_info` returns: the contract majors the running core supports. */
export interface AppInfo {
  schema: string;
  contracts: { project: number; session: number };
}

/** True when the whole string matches: never a prefix, and never "everything before a trailing newline". */
const whole = (re: RegExp, s: string): RegExpExecArray | null => {
  const m = re.exec(s);
  return m && m[0].length === s.length ? m : null;
};

const CROCKFORD = /[0-7][0-9A-HJKMNP-TV-Z]{25}/;

/** `<prefix>_` and 26 Crockford base32 characters, the first at most 7. */
export function validId(prefix: "p" | "s" | "a", id: unknown): id is string {
  return typeof id === "string" && id.startsWith(`${prefix}_`) && whole(CROCKFORD, id.slice(2)) !== null;
}

/** The major of a `vajra.<thing>/<major>` tag, or null for any other shape. */
export function schemaMajor(tag: unknown, thing: string): number | null {
  if (typeof tag !== "string") return null;
  const m = whole(new RegExp(`vajra\\.${thing}/(0|[1-9][0-9]{0,3})`), tag);
  return m ? Number(m[1]) : null;
}

const MAX_NAME_CHARS = 200;
const MAX_PATH_BYTES = 4096;
const KINDS: readonly SessionKind[] = ["orchestrator", "run", "worker", "terminal_note"];
const STATUSES: readonly SessionStatus[] = ["running", "stopped", "done", "failed"];

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === "object" && v !== null && !Array.isArray(v);
const str = (v: unknown): v is string => typeof v === "string";
const optStrOk = (v: unknown): boolean => v === undefined || v === null || str(v);
const optStr = (v: unknown): string | null => (str(v) ? v : null);
/** A u64 or u32 as the core reads it: a whole number from 0 up to `max`. (JSON has no bigger exact integers here.) */
const uint = (v: unknown, max: number): v is number => typeof v === "number" && Number.isInteger(v) && v >= 0 && v <= max;
const optUint = (v: unknown, max: number): boolean => v === undefined || v === null || uint(v, max);
const U32 = 0xffff_ffff;
/** 2^53 - 1: the contract's largest `dev`, `ino` or `rev` (the core refuses more), so JSON numbers stay exact. */
const MAX_JSON_INT = Number.MAX_SAFE_INTEGER;

type Fail = { ok: false; reason: "unsupported_version" | "malformed" | "invalid"; detail: string };
const bad = (reason: Fail["reason"], detail: string): Fail => ({ ok: false, reason, detail });

// The same invisible and reordering characters the core refuses in a name (control::lead::invisible).
// eslint-disable-next-line no-control-regex
const CONTROL = /[\u0000-\u001f\u007f-\u009f]/u;
const INVISIBLE =
  /[\u00ad\u034f\u061c\u115f\u1160\u17b4\u17b5\u180b-\u180f\u200b-\u200f\u2028-\u202e\u2060-\u206f\u3164\ufe00-\ufe0f\ufeff\uffa0\ufff0-\ufffb]|\u{1bca0}|\u{1bca1}|\u{1bca2}|\u{1bca3}|[\u{1d173}-\u{1d17a}]|[\u{e0000}-\u{e0fff}]/u;

const validName = (n: string): boolean => n.trim() !== "" && [...n].length <= MAX_NAME_CHARS && !CONTROL.test(n) && !INVISIBLE.test(n);

const byteLen = (s: string): number => new TextEncoder().encode(s).length;

/** Absolute and canonical: no `..`, `.` or empty component, no NUL, at most 4096 bytes. */
const validPath = (p: string): boolean =>
  (p === "/" || (p.startsWith("/") && p.slice(1).split("/").every((c) => c !== "" && c !== "." && c !== ".."))) && byteLen(p) <= MAX_PATH_BYTES && !p.includes("\0");

/** `YYYY-MM-DDTHH:MM:SS[.fff]Z`, as `control::time::parse_iso` reads it. */
function validTime(t: string): boolean {
  const m = whole(/(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2}):(\d{2})(?:\.\d+)?Z/, t);
  if (!m) return false;
  const [mo, d, h, mi, se] = m.slice(2, 7).map(Number);
  return mo >= 1 && mo <= 12 && d >= 1 && d <= 31 && h <= 23 && mi <= 59 && se <= 60;
}

const validProvider = (p: string): boolean => p !== "" && p.length <= 32 && whole(/[a-z0-9_-]+/, p) !== null;
const validLabel = (s: string): boolean => s !== "" && s.length <= 128 && whole(/[A-Za-z0-9_.-]+/, s) !== null && !s.startsWith(".");
const validRunId = (r: string): boolean => whole(/\d{8}-\d{4}/, r) !== null;
const validUuid = (u: string): boolean => whole(/[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}/, u) !== null;

/** Parse JSON, check size, and check the schema tag alone first (so another major reads as "update needed"). */
function open(text: string, thing: string, supported: number): { o: Record<string, unknown> } | Fail {
  if (byteLen(text) > MAX_RECORD_BYTES) return bad("malformed", "record too large");
  let o: unknown;
  try {
    o = JSON.parse(text);
  } catch {
    return bad("malformed", "not JSON");
  }
  if (!isObj(o) || !("schema" in o) || !str(o.schema)) return bad("malformed", "not a record");
  if (schemaMajor(o.schema, thing) !== supported) return bad("unsupported_version", `${o.schema.slice(0, 64)} (this build reads major ${supported})`);
  return { o };
}

const MAX_RECORD_BYTES = 64 * 1024;

/** Mirrors `parse_project`: the tag, then the shape (malformed), then the rules (invalid). */
export function parseProject(text: string): ParseResult<Project> {
  const r = open(text, "project", PROJECT_MAJOR);
  if (!("o" in r)) return r;
  const o = r.o;
  const rk = o.root_key;
  if (!str(o.id) || !str(o.name) || !str(o.root) || !str(o.created) || !uint(o.rev, Infinity) || !optStrOk(o.archived)) {
    return bad("malformed", "missing or mistyped field");
  }
  if (!isObj(rk) || !uint(rk.dev, Infinity) || !uint(rk.ino, Infinity) || !optStrOk(rk.git_common_dir)) {
    return bad("malformed", "missing or mistyped root_key");
  }
  if (!validId("p", o.id)) return bad("invalid", "project id");
  if (!validName(o.name)) return bad("invalid", "name");
  if (!validPath(o.root)) return bad("invalid", "path");
  if (str(rk.git_common_dir) && !validPath(rk.git_common_dir)) return bad("invalid", "path");
  if (!validTime(o.created)) return bad("invalid", "timestamp");
  if (str(o.archived) && !validTime(o.archived)) return bad("invalid", "timestamp");
  if (o.rev === 0 || o.rev > MAX_JSON_INT) return bad("invalid", "rev");
  if ((rk.dev as number) > MAX_JSON_INT || (rk.ino as number) > MAX_JSON_INT) return bad("invalid", "root_key");
  return {
    ok: true,
    value: {
      schema: o.schema as string,
      id: o.id,
      name: o.name,
      root: o.root,
      root_key: { dev: rk.dev as number, ino: rk.ino as number, git_common_dir: optStr(rk.git_common_dir) },
      created: o.created,
      archived: optStr(o.archived),
      rev: o.rev,
    },
  };
}

/** Mirrors `parse_session`. Only an unknown kind or status STRING reads as the catch-all; a missing or mistyped one is malformed. */
export function parseSession(text: string): ParseResult<Session> {
  const r = open(text, "session", SESSION_MAJOR);
  if (!("o" in r)) return r;
  const o = r.o;
  if (
    !str(o.id) || !str(o.project_id) || !str(o.kind) || !str(o.status) || !str(o.provider) || !str(o.started) ||
    !uint(o.generation, U32) || !optUint(o.attempt, U32) ||
    !optStrOk(o.parent) || !optStrOk(o.native_session_id) || !optStrOk(o.run_id) || !optStrOk(o.campaign) || !optStrOk(o.task_id) || !optStrOk(o.ended)
  ) {
    return bad("malformed", "missing or mistyped field");
  }
  if (!validId("s", o.id)) return bad("invalid", "session id");
  if (!validId("p", o.project_id)) return bad("invalid", "project_id");
  const kind = KINDS.find((k) => k === o.kind) ?? "other";
  const status = STATUSES.find((s) => s === o.status) ?? "unknown";
  const parent = optStr(o.parent);
  if (parent !== null && (!validId("s", parent) || parent === o.id)) return bad("invalid", "parent");
  if (kind === "orchestrator" && parent !== null) return bad("invalid", "parent");
  if (!validProvider(o.provider)) return bad("invalid", "provider");
  if (str(o.native_session_id) && !validUuid(o.native_session_id)) return bad("invalid", "native_session_id");
  if (str(o.run_id) && !validRunId(o.run_id)) return bad("invalid", "run_id");
  for (const [v, what] of [[o.campaign, "campaign"], [o.task_id, "task_id"]] as const) {
    if (str(v) && !validLabel(v)) return bad("invalid", what);
  }
  if (!validTime(o.started)) return bad("invalid", "timestamp");
  if (str(o.ended) && !validTime(o.ended)) return bad("invalid", "timestamp");
  return {
    ok: true,
    value: {
      schema: o.schema as string,
      id: o.id,
      project_id: o.project_id,
      kind,
      parent,
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
