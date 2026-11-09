import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { parseProjectProvider, parseProviderList } from "../src/providerContract";

// The same fixtures the core parses (app/core/src/providers/contract_tests.rs).
const fx = (name: string) => readFileSync(resolve(__dirname, "../../core/tests/contract", name), "utf8");

describe("provider contract fixtures", () => {
  it("reads the documented list: honest limits, a key name only", () => {
    const r = parseProviderList(fx("providers.v1.json"));
    expect(r.ok).toBe(true);
    if (!r.ok) return;
    const [c, x] = r.value.providers;
    expect([c.id, c.installed, c.tested, c.login.state, c.capabilities.stop, c.limits.length]).toEqual(["claude", true, "tested", "signed_in", "hook_block", 0]);
    expect([x.id, x.tested, x.login.state, x.login.checked_by, x.billing.key_in_app_env]).toEqual(["codex", "untested", "cannot_tell", "none", "OPENAI_API_KEY"]);
    expect(x.capabilities.message).toBe(false);
    expect(x.limits.some((l) => l.control === "message")).toBe(true);
  });

  it("reads a provider that is not installed as having no path", () => {
    const r = parseProviderList(fx("providers.v1.not-installed.json"));
    expect(r.ok && [r.value.providers[0].installed, r.value.providers[0].path, r.value.providers[0].login.state]).toEqual([false, null, "not_installed"]);
  });

  it("reads unknown states as the cautious value and ignores unknown fields", () => {
    const r = parseProviderList(fx("providers.v1.unknown-enums.json"));
    expect(r.ok && [r.value.providers[0].login.state, r.value.providers[0].tested, r.value.providers[0].capabilities.stop]).toEqual(["cannot_tell", "unknown", "other"]);
  });

  it("reports another major as unsupported, and garbage as malformed", () => {
    expect(parseProviderList(fx("providers.v2.json"))).toMatchObject({ ok: false, reason: "unsupported_version" });
    for (const bad of ["", "{", "[]", "null"]) expect(parseProviderList(bad)).toMatchObject({ ok: false, reason: "malformed" });
    expect(parseProviderList(" ".repeat(64 * 1024 + 1))).toMatchObject({ ok: false, reason: "malformed" });
  });

  it("reads the project_state provider object, with its name and tested flag", () => {
    const r = parseProjectProvider(fx("project_state.provider.v1.json"));
    expect(r.ok && [r.value.id, r.value.name, r.value.tested, r.value.installed, r.value.login_state, r.value.limits.length]).toEqual(["codex", "Codex CLI", "untested", true, "cannot_tell", 3]);
  });

  it("carries stable codes beside the fallback text, and a Stop limit for a provider that ends the process", () => {
    const r = parseProviderList(fx("providers.v1.json"));
    if (!r.ok) throw new Error("fixture");
    const x = r.value.providers[1];
    expect([x.login.route_code, x.login.detail_code]).toEqual(["uses_codex_login", "check_not_understood"]);
    expect(x.limits.find((l) => l.control === "stop")).toMatchObject({ code: "stop_ends_process", reason: "Codex has no hand-off: Stop ends its process" });
    const old = parseProviderList(fx("providers.v1.not-installed.json"));
    expect(old.ok && [old.value.providers[0].login.route_code, old.value.providers[0].limits[0].code]).toEqual([null, null]);
  });

  it("reads a chosen provider this build does not know as present, not installed and unable to start", () => {
    const r = parseProjectProvider(fx("project_state.provider.unsupported.v1.json"));
    expect(r.ok && [r.value.installed, r.value.login_state, r.value.limits.map((l) => l.code)]).toEqual([false, "not_installed", ["not_supported", "not_supported"]]);
  });
});

type Case = { case: string; at: string; set?: Record<string, unknown>; unset?: string[]; reason: string };
const patched = (base: string, c: Case): string => {
  const doc = JSON.parse(base) as Record<string, unknown>;
  let cur: any = doc;
  for (const part of c.at.split(".").filter((p) => p !== "")) cur = cur[/^\d+$/.test(part) ? Number(part) : part];
  Object.assign(cur, c.set ?? {});
  for (const k of c.unset ?? []) delete cur[k];
  return JSON.stringify(doc);
};

describe("shared invalid fixtures", () => {
  const cases = JSON.parse(fx("providers.invalid.v1.json")) as { providers: Case[]; project_provider: Case[] };
  it("rejects every provider-list case for its reason", () => {
    for (const c of cases.providers) {
      const r = parseProviderList(patched(fx("providers.v1.json"), c));
      expect({ case: c.case, ok: r.ok, reason: r.ok ? null : r.reason }).toEqual({ case: c.case, ok: false, reason: c.reason });
    }
  });
  it("rejects every project-provider case for its reason", () => {
    for (const c of cases.project_provider) {
      const r = parseProjectProvider(patched(fx("project_state.provider.v1.json"), c));
      expect({ case: c.case, ok: r.ok, reason: r.ok ? null : r.reason }).toEqual({ case: c.case, ok: false, reason: c.reason });
    }
  });
});
