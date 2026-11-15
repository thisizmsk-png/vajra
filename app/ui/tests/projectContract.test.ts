import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { parseProject, parseSession, schemaMajor, supportsContracts, validId } from "../src/projectContract";

// The same fixtures the core parses (app/core/tests/contract.rs).
const fx = (name: string) => readFileSync(resolve(__dirname, "../../core/tests/contract", name), "utf8");

describe("project contract fixtures", () => {
  it("reads the project fixture into the documented record", () => {
    const r = parseProject(fx("project.v1.json"));
    expect(r.ok).toBe(true);
    if (!r.ok) return;
    expect(r.value).toMatchObject({ id: "p_01JC5X8Z3A9K2M4N6P7Q8R9S0T", name: "vajra", root: "/Users/u/src/vajra", rev: 7, archived: null });
    expect(r.value.root_key).toEqual({ dev: "16777230", ino: "9123", git_common_dir: "/Users/u/src/vajra/.git" });
  });

  it("ignores fields it does not know", () => {
    const r = parseProject(fx("project.v1.extra-fields.json"));
    expect(r.ok && r.value.rev).toBe(1);
    expect(r.ok && r.value.root_key.git_common_dir).toBeNull();
  });

  it("reports another major version as unsupported instead of misreading it", () => {
    const r = parseProject(fx("project.v2.json"));
    expect(r).toMatchObject({ ok: false, reason: "unsupported_version" });
  });

  it("reads the session fixtures, and an unknown kind or status as the catch-all", () => {
    const s = parseSession(fx("session.v1.json"));
    expect(s.ok && [s.value.kind, s.value.status, s.value.generation, s.value.attempt]).toEqual(["worker", "running", 3, 2]);
    const u = parseSession(fx("session.v1.unknown-enums.json"));
    expect(u.ok && [u.value.kind, u.value.status]).toEqual(["other", "unknown"]);
  });

  it("treats garbage as malformed", () => {
    for (const bad of ["", "{", "[]", "null", '{"id":"x"}']) expect(parseProject(bad)).toMatchObject({ ok: false, reason: "malformed" });
  });
});

describe("contract rules", () => {
  it("validates ids the way the core does", () => {
    expect(validId("p", "p_01JC5X8Z3A9K2M4N6P7Q8R9S0T")).toBe(true);
    for (const bad of ["", "p_", "s_01JC5X8Z3A9K2M4N6P7Q8R9S0T", "p_01JC5X8Z3A9K2M4N6P7Q8R9S0I", "p_81JC5X8Z3A9K2M4N6P7Q8R9S0T", "p_01jc5x8z3a9k2m4n6p7q8r9s0t", "p_01JC5X8Z3A9K2M4N6P7Q8R9S0T\n"]) {
      expect(validId("p", bad)).toBe(false);
    }
  });

  it("reads the schema tag as a plain major", () => {
    expect(schemaMajor("vajra.project/1", "project")).toBe(1);
    for (const bad of ["vajra.project/01", "vajra.project/", "vajra.session/1", 1, null]) expect(schemaMajor(bad, "project")).toBeNull();
  });

  it("says update needed when the core speaks other majors", () => {
    expect(supportsContracts({ schema: "vajra.app_info/1", contracts: { project: 1, session: 1 } })).toBe(true);
    expect(supportsContracts({ schema: "vajra.app_info/1", contracts: { project: 2, session: 1 } })).toBe(false);
  });
});

// The invalid cases are shared with the core (tests/contract.rs): each patches a base fixture and must be rejected for
// the same reason on both sides.
type Case = { case: string; set?: Record<string, unknown>; unset?: string[]; reason: string };
const patched = (base: string, c: Case): string => {
  const v = JSON.parse(base) as Record<string, unknown>;
  Object.assign(v, c.set ?? {});
  for (const k of c.unset ?? []) delete v[k];
  return JSON.stringify(v);
};

describe("shared invalid fixtures", () => {
  const cases = JSON.parse(fx("invalid.v1.json")) as { project: Case[]; session: Case[] };
  it("rejects every project case for its reason", () => {
    for (const c of cases.project) {
      const r = parseProject(patched(fx("project.v1.json"), c));
      expect({ case: c.case, ok: r.ok, reason: r.ok ? null : r.reason }).toEqual({ case: c.case, ok: false, reason: c.reason });
    }
  });
  it("rejects every session case for its reason", () => {
    for (const c of cases.session) {
      const r = parseSession(patched(fx("session.v1.json"), c));
      expect({ case: c.case, ok: r.ok, reason: r.ok ? null : r.reason }).toEqual({ case: c.case, ok: false, reason: c.reason });
    }
  });
  it("keeps u64::MAX device and inode exact and accepts rev at 2^53-1", () => {
    const r = parseProject(fx("project.v1.max-ids.json"));
    expect(r.ok && [r.value.root_key.dev, r.value.root_key.ino, r.value.rev]).toEqual(["18446744073709551615", "9007199254740993", 9007199254740991]);
  });
  it("still accepts the valid fixtures the cases are built from", () => {
    expect(parseProject(fx("project.v1.json")).ok).toBe(true);
    expect(parseSession(fx("session.v1.json")).ok).toBe(true);
  });
  it("never lets a trailing newline through an id or a schema tag", () => {
    expect(validId("s", "s_01JC5X8Z3B0K2M4N6P7Q8R9S0T\n")).toBe(false);
    expect(schemaMajor("vajra.project/1\n", "project")).toBeNull();
    expect(schemaMajor("vajra.project/1 ", "project")).toBeNull();
  });
});

describe("project_sessions response fixture", () => {
  it("documents skipped, unsupported and truncated next to the sessions", () => {
    const r = JSON.parse(fx("project_sessions.v1.json")) as { sessions: unknown[]; skipped: number; unsupported: boolean; truncated: boolean };
    expect(Object.keys(r).sort()).toEqual(["sessions", "skipped", "truncated", "unsupported"]);
    expect([r.skipped, r.unsupported, r.truncated]).toEqual([2, true, true]);
  });
});
